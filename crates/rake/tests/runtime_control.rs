//! Covers: shared protocol, retry-safe admission, outcomes, guards, watches and application commands.
//! Real: router/task driver, generated routes, command registry and JSON wire types.
//! Doubles: explicit headless window capability. Requires: no network, GPU or browser.

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use rake::control::*;
use rake::router::*;
use serde_json::json;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, rake::Route)]
enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "document", path = "/documents/{id}")]
    Document(u64),
}

fn harness(config: RouterConfig) -> (Router<Destination>, RuntimeClient<Destination>, LocalPool) {
    let router = Router::new(config);
    let client = router.client();
    let pool = LocalPool::new();
    let driven = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driven.run().await.unwrap();
        })
        .unwrap();

    (router, client, pool)
}

fn request(
    client: &RuntimeClient<Destination>,
    key: Option<&str>,
    message: Message,
) -> Result<Reply, ControlError> {
    let request = Request {
        protocol: PROTOCOL_VERSION,
        runtime_id: client.info().runtime_id,
        key: key.map(String::from),
        message,
    };

    // Exercise the same owned representation used at both transport boundaries.
    let bytes = serde_json::to_vec(&request).unwrap();
    let reply = client.request(serde_json::from_slice(&bytes).unwrap());

    serde_json::from_slice(&serde_json::to_vec(&reply).unwrap()).unwrap()
}

fn command(client: &RuntimeClient<Destination>, key: &str, command: Command) -> Reply {
    request(client, Some(key), Message::Command(command)).unwrap()
}

fn query(client: &RuntimeClient<Destination>, query: Query) -> Reply {
    request(client, None, Message::Query(query)).unwrap()
}

#[test]
fn diagnostic_queries_preserve_cursor_kind_and_lifetime_loss_semantics() {
    let (router, client, mut pool) = harness(RouterConfig::default());

    open(&client, "diagnostic-fixture");
    router
        .app()
        .tasks()
        .spawn("fixture", async { Ok(()) })
        .unwrap();
    pool.run_until_stalled();

    let Reply::Diagnostics(all) = query(
        &client,
        Query::Diagnostics {
            after: 0,
            kind: None,
        },
    ) else {
        panic!("diagnostic snapshot expected")
    };

    assert!(all.records.iter().any(|record| record.kind == "work"));
    let after = all.records[0].sequence;

    let Reply::Diagnostics(operations) = query(
        &client,
        Query::Diagnostics {
            after,
            kind: Some("operation".into()),
        },
    ) else {
        panic!("filtered diagnostic snapshot expected")
    };

    assert!(!operations.records.is_empty());
    assert!(
        operations
            .records
            .iter()
            .all(|record| { record.sequence > after && record.kind == "operation" })
    );
    assert_eq!(operations.dropped, all.dropped);
    assert_eq!(operations.output_dropped, all.output_dropped);

    let Reply::Diagnostics(idle) = query(
        &client,
        Query::Diagnostics {
            after: all.records.last().unwrap().sequence,
            kind: None,
        },
    ) else {
        panic!("idle diagnostic snapshot expected")
    };

    assert!(idle.records.is_empty());
    assert_eq!(idle.dropped, all.dropped);
    router.shutdown();
    pool.run_until_stalled();
}

fn submitted(reply: Reply) -> OperationId {
    let Reply::Submitted { operation_id } = reply else {
        panic!("expected submission")
    };

    operation_id
}

fn open(client: &RuntimeClient<Destination>, key: &str) -> OperationId {
    submitted(command(
        client,
        key,
        Command::Open {
            window: None,
            destination: Destination::Home.encode(),
            options: OpenOptions::new_tab(),
        },
    ))
}

fn status(client: &RuntimeClient<Destination>, operation: OperationId) -> OperationRecord {
    let Reply::Operation(record) = query(client, Query::Operation { operation }) else {
        panic!()
    };

    record
}

#[test]
fn retries_share_eager_operations_and_retained_results_across_clients() {
    let (router, client, mut pool) = harness(RouterConfig::default());
    let operation = open(&client, "retry");

    assert_eq!(status(&client, operation).state, OperationState::Queued);
    assert_eq!(open(&router.client(), "retry"), operation);
    drop(client);
    pool.run_until_stalled();
    let client = router.client();
    let result = status(&client, operation);

    assert_eq!(result.state, OperationState::Committed);
    assert_eq!(router.snapshot().tabs.len(), 1);
    assert_eq!(
        result.result.unwrap()["tab_id"],
        json!(router.snapshot().tabs[0].id)
    );
    assert!(matches!(
        request(
            &client,
            Some("retry"),
            Message::Command(Command::Open {
                window: None,
                destination: Destination::Document(4).encode(),
                options: OpenOptions::new_tab()
            })
        ),
        Err(ControlError::KeyConflict)
    ));
    router.shutdown();
}

#[test]
fn admission_is_bounded_and_keys_never_silently_expire_into_duplicate_commands() {
    let (router, client, mut pool) = harness(RouterConfig {
        control_requests: 1,
        completed_operations: 0,
        ..RouterConfig::default()
    });

    let operation = open(&client, "one");

    pool.run_until_stalled();
    assert!(matches!(
        request(
            &client,
            None,
            Message::Query(Query::Operation { operation })
        ),
        Err(ControlError::Operation(OperationError::OperationExpired))
    ));
    assert_eq!(open(&client, "one"), operation);
    assert!(matches!(
        request(
            &client,
            Some("two"),
            Message::Command(Command::Close {
                tab: router.snapshot().tabs[0].id
            })
        ),
        Err(ControlError::RequestCapacity)
    ));
    assert_eq!(router.snapshot().tabs.len(), 1);
    router.shutdown();
}

#[test]
fn decisions_survive_disconnect_and_stale_responses_never_approve_changed_entries() {
    let (router, client, mut pool) = harness(RouterConfig::default());

    open(&client, "open");
    pool.run_until_stalled();
    let snapshot = router.snapshot();
    let tab = snapshot.tabs[0].id;
    let entry = router.entry(snapshot.tabs[0].entries[0].id).unwrap();

    entry.set_dirty(true).unwrap();
    let operation = submitted(command(&client, "close", Command::Close { tab }));

    pool.run_until_stalled();
    let decision = router.snapshot().decisions[0].clone();

    assert_eq!(
        status(&client, operation).state,
        OperationState::AwaitingDecision(decision.id)
    );
    drop(client);
    pool.run_until_stalled();
    assert_eq!(router.snapshot().tabs.len(), 1);
    let client = router.client();

    entry.set_dirty(false).unwrap();
    assert!(matches!(
        request(
            &client,
            Some("obsolete"),
            Message::Command(Command::Respond {
                decision: decision.id,
                choice: "discard".into()
            })
        ),
        Err(ControlError::Decision(DecisionError::Obsolete))
    ));
    pool.run_until_stalled();
    assert_eq!(status(&client, operation).state, OperationState::Committed);
    assert!(router.snapshot().tabs.is_empty());
    router.shutdown();
}

#[test]
fn cli_choices_cancel_and_commit_the_same_guarded_operations() {
    let (router, client, mut pool) = harness(RouterConfig::default());

    open(&client, "open");
    pool.run_until_stalled();
    let tab = router.snapshot().tabs[0].id;

    router
        .entry(router.snapshot().tabs[0].entries[0].id)
        .unwrap()
        .set_dirty(true)
        .unwrap();
    let close = submitted(command(&client, "close", Command::Close { tab }));

    pool.run_until_stalled();
    let decision = router.snapshot().decisions[0].id;

    command(
        &client,
        "cancel-choice",
        Command::Respond {
            decision,
            choice: "cancel".into(),
        },
    );
    pool.run_until_stalled();
    assert!(matches!(
        status(&client, close).state,
        OperationState::Cancelled(_)
    ));
    let close = submitted(command(&client, "close-again", Command::Close { tab }));

    pool.run_until_stalled();
    let decision = router.snapshot().decisions[0].id;

    command(
        &client,
        "discard-choice",
        Command::Respond {
            decision,
            choice: "discard".into(),
        },
    );
    pool.run_until_stalled();
    assert_eq!(status(&client, close).state, OperationState::Committed);
    assert!(router.snapshot().tabs.is_empty());
    router.shutdown();
}

#[test]
fn atomic_watch_sees_changes_before_await_and_reports_revision_and_epoch_gaps() {
    let (router, client, mut pool) = harness(RouterConfig::default());
    let mut watch = client.watch();

    let initial = EventCursor {
        runtime_id: client.info().runtime_id,
        epoch: 1,
        revision: watch.snapshot().revision,
    };

    open(&client, "one");
    pool.run_until_stalled();
    let event = pool.run_until(watch.next()).unwrap();

    assert_eq!(event.kind, EventKind::Gap);
    assert_eq!(event.snapshot.unwrap().tabs.len(), 1);

    let Reply::Event(event) = query(
        &client,
        Query::Events {
            cursor: Some(event.cursor),
        },
    ) else {
        panic!()
    };

    assert_eq!(event.kind, EventKind::Unchanged);

    let Reply::Event(event) = query(
        &client,
        Query::Events {
            cursor: Some(EventCursor {
                epoch: 2,
                ..initial
            }),
        },
    ) else {
        panic!()
    };

    assert_eq!(event.kind, EventKind::Gap);
    assert!(event.snapshot.is_some());
    router.shutdown();
}

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "counter.add", output = u64, capability = Headless)]
#[serde(deny_unknown_fields)]
struct Add {
    value: u64,
}

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "window.only")]
struct WindowOnly;

#[test]
fn typed_commands_are_local_eager_observed_and_retry_safe_with_explicit_capabilities() {
    let (router, client, mut pool) = harness(RouterConfig::default());
    let count = Rc::new(Cell::new(0));
    let owned = count.clone();

    client
        .commands()
        .register::<Add, _, _>(move |command, _| {
            let count = owned.clone();

            async move {
                count.set(count.get() + command.value);

                Ok(count.get())
            }
        })
        .unwrap();
    client
        .commands()
        .register::<WindowOnly, _, _>(|_, _| async { panic!("must not invoke UI handler") })
        .unwrap();
    let operation = submitted(client.dispatch(Add { value: 7 }, "typed".into()).unwrap());

    assert_eq!(count.get(), 0);
    assert_eq!(
        submitted(client.dispatch(Add { value: 7 }, "typed".into()).unwrap()),
        operation
    );
    pool.run_until_stalled();
    assert_eq!(count.get(), 7);
    assert_eq!(status(&client, operation).result, Some(json!(7)));
    assert!(matches!(
        client.dispatch(WindowOnly, "window".into()),
        Err(ControlError::Command(CommandError::RequiresWindow))
    ));
    assert!(matches!(
        request(
            &client,
            Some("bad-args"),
            Message::Command(Command::Run {
                id: "counter.add".into(),
                arguments: json!({"value":"bad"})
            })
        ),
        Err(ControlError::Command(CommandError::Arguments(_)))
    ));
    router.shutdown();
}

#[test]
fn queued_and_running_application_commands_cancel_without_starting_or_retaining_work() {
    let (router, client, mut pool) = harness(RouterConfig::default());
    let invoked = Rc::new(Cell::new(false));
    let owned = invoked.clone();

    client
        .commands()
        .register::<Add, _, _>(move |_, _| {
            owned.set(true);

            async { futures::future::pending().await }
        })
        .unwrap();
    let queued = submitted(client.dispatch(Add { value: 1 }, "queued".into()).unwrap());

    command(
        &client,
        "cancel-queued",
        Command::CancelOperation { operation: queued },
    );
    pool.run_until_stalled();
    assert!(!invoked.get());
    assert!(matches!(
        status(&client, queued).state,
        OperationState::Cancelled(_)
    ));
    let running = submitted(client.dispatch(Add { value: 1 }, "running".into()).unwrap());

    pool.run_until_stalled();
    assert!(invoked.get());
    command(
        &client,
        "cancel-running",
        Command::CancelOperation { operation: running },
    );
    pool.run_until_stalled();
    assert!(matches!(
        status(&client, running).state,
        OperationState::Cancelled(_)
    ));
    router.shutdown();
}

#[test]
fn wire_validation_rejects_wrong_runtime_version_unknown_fields_and_missing_keys() {
    let (router, client, _) = harness(RouterConfig::default());
    let other = Router::<Destination>::new(RouterConfig::default());

    assert!(matches!(
        client.request(Request {
            protocol: 999,
            runtime_id: client.info().runtime_id,
            key: None,
            message: Message::Query(Query::Info {})
        }),
        Err(ControlError::ProtocolVersion { .. })
    ));
    assert!(matches!(
        client.request(Request {
            protocol: 1,
            runtime_id: other.snapshot().runtime_id,
            key: None,
            message: Message::Query(Query::Info {})
        }),
        Err(ControlError::WrongRuntime)
    ));
    assert!(matches!(
        request(
            &client,
            None,
            Message::Command(Command::Close {
                tab: "tab-1".parse().unwrap()
            })
        ),
        Err(ControlError::InvalidRequestKey)
    ));
    assert!(
        serde_json::from_value::<Message>(
            json!({"type":"query", "value":{"type":"info", "exec":"bad"}})
        )
        .is_err()
    );
    router.shutdown();
    assert!(matches!(
        request(&client, None, Message::Query(Query::Info {})),
        Err(ControlError::RuntimeShutdown)
    ));
}

#[test]
fn cancellation_precedes_a_simultaneously_ready_command_completion() {
    let (router, client, mut pool) = harness(RouterConfig::default());
    let (sender, receiver) = futures::channel::oneshot::channel::<()>();
    let receiver = Rc::new(std::cell::RefCell::new(Some(receiver)));
    let committed = Rc::new(Cell::new(false));
    let owned = committed.clone();

    client
        .commands()
        .register::<Add, _, _>(move |_, _| {
            let receiver = receiver.borrow_mut().take().unwrap();
            let committed = owned.clone();

            async move {
                receiver.await.unwrap();
                committed.set(true);

                Ok(1)
            }
        })
        .unwrap();
    let operation = submitted(client.dispatch(Add { value: 1 }, "ready".into()).unwrap());

    pool.run_until_stalled();
    sender.send(()).unwrap();
    command(
        &client,
        "cancel-ready",
        Command::CancelOperation { operation },
    );
    pool.run_until_stalled();
    assert!(
        !committed.get(),
        "a cancellation admitted before completion must win before polling ready work"
    );
    assert!(matches!(
        status(&client, operation).state,
        OperationState::Cancelled(_)
    ));
    router.shutdown();
}

#[test]
fn cancellation_precedes_a_ready_navigation_guard_commit() {
    let (router, client, mut pool) = harness(RouterConfig::default());
    let (sender, receiver) = futures::channel::oneshot::channel::<()>();
    let receiver = Rc::new(std::cell::RefCell::new(Some(receiver)));

    router
        .entry_guard(move |_| {
            let receiver = receiver.borrow_mut().take().unwrap();

            async move {
                receiver.await.unwrap();

                GuardResult::Allow
            }
        })
        .unwrap();
    let operation = open(&client, "ready-navigation");

    pool.run_until_stalled();
    sender.send(()).unwrap();
    command(
        &client,
        "cancel-ready-navigation",
        Command::CancelOperation { operation },
    );
    pool.run_until_stalled();
    assert!(router.snapshot().tabs.is_empty());
    assert!(matches!(
        status(&client, operation).state,
        OperationState::Cancelled(_)
    ));
    router.shutdown();
}

#[test]
fn queue_rejection_never_invokes_application_handlers_and_failures_are_queryable() {
    let (router, client, mut pool) = harness(RouterConfig {
        pending_operations: 0,
        ..RouterConfig::default()
    });

    client
        .commands()
        .register::<Add, _, _>(|_, _| {
            panic!("a rejected command must not invoke the handler");

            #[allow(unreachable_code)]
            async {
                Ok(0)
            }
        })
        .unwrap();
    let operation = submitted(client.dispatch(Add { value: 1 }, "full".into()).unwrap());

    assert_eq!(
        status(&client, operation).state,
        OperationState::Failed(OperationError::QueueFull)
    );
    pool.run_until_stalled();
    router.shutdown();

    let (router, client, mut pool) = harness(RouterConfig::default());

    client
        .commands()
        .register::<Add, _, _>(|_, _| async {
            Err(CommandError::Failed("disk unavailable".into()))
        })
        .unwrap();
    let operation = submitted(client.dispatch(Add { value: 1 }, "failure".into()).unwrap());

    pool.run_until_stalled();
    assert!(matches!(
        status(&client, operation).state,
        OperationState::Failed(OperationError::Command(CommandError::Failed(_)))
    ));
    assert!(status(&client, operation).result.is_none());
    router.shutdown();
}
