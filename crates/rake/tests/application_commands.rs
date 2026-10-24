//! Covers: derived metadata, schema contracts and one GUI/wire command dispatcher.
//! Real: facade macros, Schemars/Serde, registry, local router driver and operation retention.
//! Doubles: headless route mounts. Requires: no GPU, browser or network.

use std::cell::Cell;
use std::rc::Rc;

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;

use rake::control::{
    ApplicationCommand, Command, CommandError, ControlError, ExecutionCapability, Message,
    PROTOCOL_VERSION, Query, Reply, Request,
};

use rake::router::{NoRoute, OperationState, Router, RouterConfig};
use serde_json::json;

#[rake::data(schema)]
struct DocumentId(u64);

#[rake::data(schema)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Summary,
    Detail,
}

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "document.describe", output = Description, capability = Headless)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Describe {
    document_id: DocumentId,
    #[serde(default = "default_limit")]
    limit: u16,
    mode: Option<Mode>,
    #[serde(default)]
    tags: Vec<String>,
}

fn default_limit() -> u16 {
    10
}

#[rake::data(schema)]
#[serde(rename_all = "camelCase")]
struct Description {
    document_id: DocumentId,
    limit: u16,
    #[serde(skip_serializing)]
    internal: String,
    #[serde(skip_deserializing)]
    server_stamp: bool,
}

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "window.refresh")]
struct Refresh;

#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "tuple.echo", output = Vec<String>, capability = Headless)]
struct Tuple(String, u32);

#[test]
fn schemas_follow_nested_types_defaults_renames_and_distinct_result_contracts() {
    let descriptor = Describe::descriptor();

    assert_eq!(descriptor.id, Describe::id());
    assert_eq!(descriptor.capability, ExecutionCapability::Headless);
    assert_eq!(
        descriptor.arguments["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(descriptor.arguments["additionalProperties"], false);
    assert_eq!(descriptor.arguments["required"], json!(["documentId"]));
    assert_eq!(descriptor.arguments["properties"]["limit"]["default"], 10);
    assert_eq!(
        descriptor.arguments["properties"]["limit"]["maximum"],
        65535
    );
    assert_eq!(
        descriptor.arguments["properties"]["tags"]["items"]["type"],
        "string"
    );
    assert_eq!(
        descriptor.arguments["$defs"]["DocumentId"]["type"],
        "integer"
    );
    assert_eq!(
        descriptor.arguments["$defs"]["Mode"]["enum"],
        json!(["summary", "detail"])
    );
    assert!(descriptor.result["properties"].get("internal").is_none());
    assert_eq!(
        descriptor.result["properties"]["serverStamp"]["type"],
        "boolean"
    );
    assert_eq!(
        Refresh::descriptor().capability,
        ExecutionCapability::RequiresWindow
    );
    assert_eq!(Refresh::descriptor().result["type"], "null");
    assert_eq!(Tuple::descriptor().arguments["type"], "array");
    assert_eq!(Tuple::descriptor().arguments["minItems"], 2);
    assert_eq!(Tuple::descriptor().result["items"]["type"], "string");
}

fn request(router: &Router<NoRoute>, message: Message, key: Option<&str>) -> Request {
    Request {
        protocol: PROTOCOL_VERSION,
        runtime_id: router.runtime_id(),
        key: key.map(str::to_owned),
        message,
    }
}

#[test]
fn derived_gui_and_wire_requests_share_admission_results_and_retry_identity() {
    let router = Router::<NoRoute>::new(RouterConfig::default());
    let client = router.client();
    let calls = Rc::new(Cell::new(0));
    let recorded = calls.clone();

    client
        .commands()
        .register::<Describe, _, _>(move |args, context| {
            let calls = recorded.clone();

            async move {
                assert!(!context.cancellation.is_cancelled());
                calls.set(calls.get() + 1);
                let result = Description {
                    document_id: args.document_id,
                    limit: args.limit,
                    internal: "private".into(),
                    server_stamp: true,
                };

                assert_eq!(result.internal, "private");

                Ok(result)
            }
        })
        .unwrap();
    assert!(matches!(
        client
            .commands()
            .register::<Describe, _, _>(|_, _| async { unreachable!() }),
        Err(CommandError::Registration(_))
    ));
    client
        .commands()
        .register::<Refresh, _, _>(|_, _| async {
            panic!("headless host must reject before invocation")
        })
        .unwrap();

    let mut pool = LocalPool::new();
    let driven = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driven.run().await.unwrap();
        })
        .unwrap();
    let arguments = Describe {
        document_id: DocumentId(42),
        limit: 10,
        mode: None,
        tags: vec![],
    };

    // Navigator is the weak proxy used by PageContext in GUI listeners.
    let submitted = router
        .navigator()
        .dispatch(arguments, "one-action".into())
        .unwrap();

    let wire = request(
        &router,
        Message::Command(Command::Run {
            id: "document.describe".into(),
            arguments: json!({"documentId":42, "limit":10, "mode":null, "tags":[]}),
        }),
        Some("one-action"),
    );

    let retried = client
        .request(serde_json::from_slice(&serde_json::to_vec(&wire).unwrap()).unwrap())
        .unwrap();

    let Reply::Submitted { operation_id } = submitted else {
        panic!("submission");
    };

    assert_eq!(
        serde_json::to_value(retried).unwrap(),
        json!({"type":"submitted","value":{"operation_id":operation_id}})
    );
    assert_eq!(calls.get(), 0);
    pool.run_until_stalled();
    assert_eq!(calls.get(), 1);

    let Reply::Operation(record) = client
        .request(request(
            &router,
            Message::Query(Query::Operation {
                operation: operation_id,
            }),
            None,
        ))
        .unwrap()
    else {
        panic!("operation");
    };

    assert!(matches!(record.state, OperationState::Committed));
    assert_eq!(
        record.result.unwrap(),
        json!({"documentId":42,"limit":10,"serverStamp":true})
    );

    for (key, args) in [
        ("unknown-field", json!({"documentId":42,"typo":true})),
        ("negative", json!({"documentId":-1})),
        ("range", json!({"documentId":42,"limit":65536})),
        ("bad-enum", json!({"documentId":42,"mode":"not-a-mode"})),
    ] {
        assert!(matches!(
            client.request(request(
                &router,
                Message::Command(Command::Run {
                    id: "document.describe".into(),
                    arguments: args
                }),
                Some(key)
            )),
            Err(ControlError::Command(CommandError::Arguments(_)))
        ));
    }

    assert!(matches!(
        client.dispatch(Refresh, "window".into()),
        Err(ControlError::Command(CommandError::RequiresWindow))
    ));
    assert_eq!(calls.get(), 1);
    // Omitted fields use exactly the defaults advertised by the argument schema.
    client
        .request(request(
            &router,
            Message::Command(Command::Run {
                id: "document.describe".into(),
                arguments: json!({"documentId":7}),
            }),
            Some("defaults"),
        ))
        .unwrap();
    pool.run_until_stalled();
    assert_eq!(calls.get(), 2);
    router.shutdown();
}

#[cfg(feature = "gpui")]
#[test]
fn application_builder_rejects_duplicate_derived_commands_before_starting_a_window() {
    let app = rake::App::new("test.commands")
        .command::<Refresh, _, _>(|_, _| async { Ok(()) })
        .unwrap();

    assert!(matches!(
        app.command::<Refresh, _, _>(|_, _| async { Ok(()) }),
        Err(CommandError::Registration(_))
    ));
}
