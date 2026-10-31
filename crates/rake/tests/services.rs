//! Covers: typed service composition, stable shared ownership and command resolution.
//! Real: public registries, application services and headless runtime dispatch.
//! Doubles: none. Requires: no display or network.

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;
use rake::prelude::*;
use rake::router::NoRoute;
use std::cell::Cell;
use std::rc::Rc;

struct Counter(Cell<u64>);

struct Label(String);

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "counter.add", output = u64, capability = Headless)]
struct Add {
    amount: u64,
}

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "services.standard", output = bool, capability = Headless)]
struct InspectStandard;

#[test]
fn snapshots_share_values_and_never_replace_a_registered_type() {
    let services = ServiceRegistry::new().with(Counter(Cell::new(1))).unwrap();
    let previous = services.clone();
    let services = services.with(Label("workspace".into())).unwrap();

    assert!(matches!(
        previous.get::<Label>(),
        Err(ServiceError::Missing(_))
    ));
    assert_eq!(services.get::<Label>().unwrap().0, "workspace");
    assert!(Rc::ptr_eq(
        &services.get::<Counter>().unwrap(),
        &previous.get::<Counter>().unwrap()
    ));
    assert!(matches!(
        services.with(Counter(Cell::new(0))),
        Err(ServiceError::Duplicate(_))
    ));
}

#[test]
fn standard_services_are_installed_with_shared_transport_identity() {
    let collector = Collector::default();
    let http = HttpClient::with_collector(HttpConfig::new(), collector.clone()).unwrap();
    let custom = ServiceRegistry::new().with(Counter(Cell::new(1))).unwrap();
    let services = Services::new(http.clone(), collector, custom);
    let registry = services.registry();

    let registry_http = registry.http().unwrap();
    assert!(http.shares_transport_with(&registry_http));

    let reconstructed = registry.standard_services().unwrap();
    assert!(http.shares_transport_with(&reconstructed.http()));
    assert!(Rc::ptr_eq(
        &services.get::<Counter>().unwrap(),
        &reconstructed.get::<Counter>().unwrap()
    ));
}

#[test]
fn from_config_builds_a_coherent_standard_service_set() {
    let services = Services::from_config(
        HttpConfig::new(),
        Collector::default(),
        ServiceRegistry::new(),
    )
    .unwrap();

    assert!(
        services
            .http()
            .shares_transport_with(&services.registry().http().unwrap())
    );
}

#[test]
fn raw_router_service_registry_reports_missing_standard_services() {
    let registry = ServiceRegistry::new();

    assert!(matches!(
        registry.http(),
        Err(ServiceError::Missing(name)) if name.contains("HttpClient")
    ));
    assert!(matches!(
        registry.diagnostics(),
        Err(ServiceError::Missing(name)) if name.contains("Collector")
    ));
    assert!(matches!(
        registry.standard_services(),
        Err(ServiceError::Missing(_))
    ));

    let router = Router::<NoRoute>::with_services(RouterConfig::default(), registry);
    assert!(matches!(
        router.client().services().http(),
        Err(ServiceError::Missing(_))
    ));
    router.shutdown();
}

#[test]
fn native_and_headless_commands_receive_the_same_application_values() {
    let services = ServiceRegistry::new().with(Counter(Cell::new(1))).unwrap();
    let counter = services.get::<Counter>().unwrap();
    let router = Router::<NoRoute>::with_services(RouterConfig::default(), services);
    let client = router.client();

    client
        .commands()
        .register::<Add, _, _>(|args, context| async move {
            assert!(matches!(context.http(), Err(ServiceError::Missing(_))));

            let counter = context
                .services
                .get::<Counter>()
                .map_err(|error| CommandError::Failed(error.to_string()))?;

            counter.0.set(counter.0.get() + args.amount);

            Ok(counter.0.get())
        })
        .unwrap();
    let reply = router
        .navigator()
        .dispatch(Add { amount: 4 }, "gui".into())
        .unwrap();

    let rake::control::Reply::Submitted { operation_id } = reply else {
        panic!()
    };

    let mut pool = LocalPool::new();
    let driver = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driver.run().await.unwrap();
        })
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(counter.0.get(), 5);
    assert!(Rc::ptr_eq(
        &counter,
        &client.services().get::<Counter>().unwrap()
    ));
    let reply = client
        .request(rake::control::Request {
            protocol: rake::control::PROTOCOL_VERSION,
            runtime_id: router.runtime_id(),
            key: None,
            message: rake::control::Message::Query(rake::control::Query::Operation {
                operation: operation_id,
            }),
        })
        .unwrap();

    assert!(
        matches!(reply, rake::control::Reply::Operation(record) if record.result == Some(rake::serde_json::json!(5)))
    );
    router.shutdown();
}

#[test]
fn headless_commands_receive_the_standard_service_handles() {
    let collector = Collector::default();
    let http = HttpClient::with_collector(HttpConfig::new(), collector.clone()).unwrap();
    let services = Services::new(http.clone(), collector, ServiceRegistry::new());
    let router = Router::<NoRoute>::with_services(RouterConfig::default(), services.registry());
    let client = router.client();
    let observed = Rc::new(Cell::new(false));
    let observed_handler = observed.clone();
    let expected = http.clone();

    client
        .commands()
        .register::<InspectStandard, _, _>(move |_args, context| {
            let expected = expected.clone();
            let observed = observed_handler.clone();

            async move {
                let actual = context
                    .http()
                    .map_err(|error| CommandError::Failed(error.to_string()))?;
                observed.set(actual.shares_transport_with(&expected));
                assert!(context.app_services().is_ok());

                Ok(true)
            }
        })
        .unwrap();

    let reply = router
        .navigator()
        .dispatch(InspectStandard, "headless".into())
        .unwrap();
    assert!(matches!(reply, rake::control::Reply::Submitted { .. }));

    let mut pool = LocalPool::new();
    let driver = router.clone();
    pool.spawner()
        .spawn_local(async move {
            driver.run().await.unwrap();
        })
        .unwrap();
    pool.run_until_stalled();

    assert!(observed.get());
    router.shutdown();
}

#[cfg(feature = "gpui")]
#[test]
fn application_configuration_exposes_the_registered_service_before_startup() {
    let app = App::new("test.services")
        .service(Counter(Cell::new(2)))
        .unwrap();

    let first_services = app.services().unwrap();
    let second_services = app.services().unwrap();
    let first = first_services.get::<Counter>().unwrap();
    let second = second_services.get::<Counter>().unwrap();

    first.0.set(9);
    assert!(Rc::ptr_eq(&first, &second));
    assert!(
        first_services
            .http()
            .shares_transport_with(&second_services.http())
    );
    assert_eq!(second.0.get(), 9);
    assert!(matches!(
        app.service(Counter(Cell::new(0))),
        Err(ServiceError::Duplicate(_))
    ));
}
