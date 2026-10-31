//! Covers: public headless runtime ownership, commands, tasks and guarded quit.
//! Real: `Runtime`, `RuntimeContext`, the local driver and application services.
//! Doubles: local cells and a no-op headless HTTP transport; no UI or network.
//! Requires: no display, GPU, browser or native host.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use rake::control::{CommandError, ControlError, Message, PROTOCOL_VERSION, Query, Reply, Request};
use rake::prelude::*;
use rake::router::OperationError;
use rake::runtime::Runtime;
use rake::tasks::TaskState;

struct Counter(Cell<u64>);

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "runtime.increment", output = u64, capability = Headless)]
struct Increment {
    amount: u64,
}

fn services(counter: Counter) -> Services {
    let registry = ServiceRegistry::new().with(counter).unwrap();
    Services::new(
        HttpClient::new(HttpConfig::new()).unwrap(),
        Collector::default(),
        registry,
    )
}

fn drive(runtime: Rc<Runtime>, finished: Rc<Cell<bool>>) -> LocalPool {
    let pool = LocalPool::new();
    pool.spawner()
        .spawn_local(async move {
            runtime.run().await.unwrap();
            finished.set(true);
        })
        .unwrap();
    pool
}

#[test]
fn runtime_context_commands_and_tasks_share_services() {
    let runtime = Rc::new(Runtime::new(
        RouterConfig::default(),
        services(Counter(Cell::new(1))),
    ));
    let context = runtime.context();
    let counter = context.services().get::<Counter>().unwrap();
    let command_counter = counter.clone();

    context
        .client()
        .unwrap()
        .commands()
        .register::<Increment, _, _>(move |args, command_context| {
            let command_counter = command_counter.clone();
            async move {
                let service_counter = command_context
                    .services
                    .get::<Counter>()
                    .map_err(|error| CommandError::Failed(error.to_string()))?;
                assert!(Rc::ptr_eq(&command_counter, &service_counter));
                service_counter.0.set(service_counter.0.get() + args.amount);
                Ok(service_counter.0.get())
            }
        })
        .unwrap();

    let reply = context
        .client()
        .unwrap()
        .dispatch(Increment { amount: 4 }, "increment".into())
        .unwrap();
    assert!(matches!(reply, Reply::Submitted { .. }));

    let finished = Rc::new(Cell::new(false));
    let mut pool = drive(runtime.clone(), finished.clone());
    pool.run_until_stalled();
    assert_eq!(counter.0.get(), 5);

    let completed = Rc::new(Cell::new(false));
    let completed_by_task = completed.clone();
    let task = context
        .tasks()
        .spawn("headless-task", async move {
            completed_by_task.set(true);
            Ok(())
        })
        .unwrap();
    pool.run_until_stalled();
    assert!(completed.get());
    assert_eq!(task.snapshot().unwrap().state, TaskState::Completed);

    runtime.shutdown();
    pool.run_until_stalled();
    assert!(finished.get());
}

#[test]
fn committed_quit_auto_stops_the_headless_runtime_after_guards() {
    let runtime = Rc::new(Runtime::new(
        RouterConfig::default(),
        services(Counter(Cell::new(0))),
    ));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let observed = calls.clone();
    runtime
        .quit_guard(move |context| {
            observed.borrow_mut().push(context.operation_id);
            async { Ok(()) }
        })
        .unwrap();

    let context = runtime.context();
    let finished = Rc::new(Cell::new(false));
    let mut pool = drive(runtime.clone(), finished.clone());
    let quit = runtime.quit(QuitOptions::default());
    let quit_id = quit.id();

    assert!(matches!(
        pool.run_until(quit.wait()).unwrap(),
        Outcome::Committed(_)
    ));
    pool.run_until_stalled();

    assert_eq!(&*calls.borrow(), &[quit_id]);
    assert_eq!(runtime.quit_status().phase, QuitPhase::Ready);
    assert!(finished.get());
    assert!(matches!(
        context.client(),
        Err(OperationError::RuntimeShutdown)
    ));
}

#[test]
fn dropping_the_owner_expires_retained_contexts_but_services_remain_readable() {
    let runtime = Runtime::new(RouterConfig::default(), services(Counter(Cell::new(7))));
    let context = runtime.context();
    let client = context.client().unwrap();
    assert_eq!(context.services().get::<Counter>().unwrap().0.get(), 7);

    drop(runtime);

    assert!(matches!(
        context.client(),
        Err(OperationError::RuntimeShutdown)
    ));
    assert!(matches!(
        client.request(Request {
            protocol: PROTOCOL_VERSION,
            runtime_id: client.info().runtime_id,
            key: None,
            message: Message::Query(Query::Info {}),
        }),
        Err(ControlError::RuntimeShutdown)
    ));
    assert!(matches!(
        context.quit_status(),
        Err(OperationError::RuntimeShutdown)
    ));
    let mut pool = LocalPool::new();
    assert!(matches!(
        pool.run_until(context.quit(QuitOptions::default()).wait()),
        Err(OperationError::RuntimeShutdown)
    ));
    assert_eq!(context.services().get::<Counter>().unwrap().0.get(), 7);
}
