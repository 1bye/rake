//! Covers: explicit registration, read-only enforcement, typed edits, retry keys,
//! cache invalidation, cursor/byte bounds and stale runtime rejection.
//! Real: typed stores, cache, registry, Serde and routed async host dispatch.
//! Doubles: none.
//! Requires: the devtools feature; no GPU or external services.

#![cfg(feature = "devtools")]

use futures::executor::block_on;
use rake::{
    control::{
        Command, ControlError, Message, PROTOCOL_VERSION, Query, Reply, Request, RuntimeClient,
    },
    router::{NoRoute, Router, RouterConfig},
    services::ServiceRegistry,
    storage::{
        CacheConfig, MemoryBackend, Store,
        devtools::{Area, Storage},
    },
};
use rake_protocol::storage::{Edit, Page, Read};

fn request(client: &RuntimeClient<NoRoute>, message: Message, key: Option<&str>) -> Request {
    Request {
        protocol: PROTOCOL_VERSION,
        runtime_id: client.info().runtime_id,
        message,
        key: key.map(str::to_owned),
    }
}
fn edit(value: Option<&str>) -> Message {
    Message::Command(Command::EditStorage {
        edit: Edit {
            store: "preferences".into(),
            key: "theme".into(),
            value: value.map(str::to_owned),
        },
    })
}

#[test]
fn registered_typed_edits_permissions_and_retry_lifetimes() {
    block_on(async {
        let store = Store::<String>::new(MemoryBackend::new())
            .with_cache(CacheConfig::new(8, 1024).unwrap());
        store.set("theme", &"dark".into()).await.unwrap();
        assert_eq!(store.get("theme").await.unwrap().as_deref(), Some("dark"));
        let registry = Storage::new()
            .read_write("preferences", Area::Local, store.clone())
            .unwrap()
            .read_only("session", Area::Session, store.clone())
            .unwrap();
        assert!(
            registry
                .clone()
                .read_only("preferences", Area::Local, store.clone())
                .is_err()
        );
        assert!(
            Storage::new()
                .read_only("bad/name", Area::Local, store.clone())
                .is_err()
        );
        let router = Router::<NoRoute>::with_services(
            RouterConfig::default(),
            ServiceRegistry::new().with(registry).unwrap(),
        );
        let client = router.client();
        let first = request(&client, edit(Some("\"light\"")), Some("edit-1"));
        assert!(matches!(
            client.request_async(first.clone()).await.unwrap(),
            Reply::Acknowledged
        ));
        assert_eq!(store.get("theme").await.unwrap().as_deref(), Some("light"));
        store.set("theme", &"app changed".into()).await.unwrap();
        client.request_async(first).await.unwrap();
        assert_eq!(
            store.get("theme").await.unwrap().as_deref(),
            Some("app changed"),
            "retry cannot repeat a completed write"
        );
        assert!(matches!(
            client
                .request_async(request(&client, edit(None), Some("edit-1")))
                .await,
            Err(ControlError::KeyConflict)
        ));
        assert!(matches!(
            client
                .request_async(request(&client, edit(Some("3")), Some("invalid")))
                .await,
            Err(ControlError::Storage { .. })
        ));
        assert_eq!(
            store.get("theme").await.unwrap().as_deref(),
            Some("app changed")
        );
        let readonly = Message::Command(Command::EditStorage {
            edit: Edit {
                store: "session".into(),
                key: "theme".into(),
                value: None,
            },
        });
        assert!(matches!(
            client
                .request_async(request(&client, readonly, Some("readonly")))
                .await,
            Err(ControlError::Storage { .. })
        ));
        assert!(matches!(
            client
                .request_async(request(&client, edit(None), None))
                .await,
            Err(ControlError::InvalidRequestKey)
        ));
        let mut wrong = request(&client, edit(None), Some("wrong"));
        wrong.runtime_id = Router::<NoRoute>::new(RouterConfig::default()).runtime_id();
        assert!(matches!(
            client.request_async(wrong).await,
            Err(ControlError::WrongRuntime)
        ));
        let mut wrong = request(&client, edit(None), Some("wrong"));
        wrong.protocol += 1;
        assert!(matches!(
            client.request_async(wrong).await,
            Err(ControlError::ProtocolVersion { .. })
        ));
        client
            .request_async(request(&client, edit(None), Some("remove")))
            .await
            .unwrap();
        assert!(store.get("theme").await.unwrap().is_none());
    });
}

#[test]
fn large_pages_continue_without_losing_keys() {
    block_on(async {
        let store = Store::<String>::new(MemoryBackend::new());
        for key in ["a", "b", "c"] {
            store.set(key, &"x".repeat(200 * 1024)).await.unwrap();
        }
        let registry = Storage::new()
            .read_only("large", Area::Local, store)
            .unwrap();
        let router = Router::<NoRoute>::with_services(
            RouterConfig::default(),
            ServiceRegistry::new().with(registry).unwrap(),
        );
        let client = router.client();
        let mut after = None;
        let mut keys = Vec::new();
        loop {
            let message = Message::Query(Query::Storage {
                read: Read::Entries {
                    store: "large".into(),
                    after,
                    limit: 256,
                },
            });
            let Reply::Data(data) = client
                .request_async(request(&client, message, None))
                .await
                .unwrap()
            else {
                panic!("expected page");
            };
            let page: Page = serde_json::from_value(data).unwrap();
            assert_eq!(page.entries.len(), 1);
            keys.extend(page.entries.into_iter().map(|(key, _)| key));
            let Some(next) = page.next else {
                break;
            };
            after = Some(next);
        }
        assert_eq!(keys, ["a", "b", "c"]);
    });
}
