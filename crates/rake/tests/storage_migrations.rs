//! Covers: schema envelopes, migration chains, failure preservation, fixtures and concurrent handles.
//! Real: typed stores, schema codec, cache, memory and native file backends.
//! Doubles: a counting backend with manually released I/O and injected write failures.
//! Requires: a writable temporary directory for native file tests; no display/network.

use futures::{channel::oneshot, executor::block_on, future::LocalBoxFuture};
use rake::storage::{Backend, CacheConfig, MemoryBackend, Schema, StorageError, Store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Note {
    title: String,
    pinned: bool,
}

fn schema() -> Schema {
    Schema::new(2)
        .with_migration(0, |mut value| {
            let title = value
                .as_object_mut()
                .and_then(|object| object.remove("name"))
                .ok_or(StorageError::MissingMigration { from: 0 })?;

            value["title"] = title;
            Ok(value)
        })
        .unwrap()
        .with_migration(1, |mut value| {
            value["pinned"] = json!(false);
            Ok(value)
        })
        .unwrap()
}

async fn backend_contract(backend: impl Backend + Clone) {
    let raw = br#"{ "name": "old" }"#;

    backend.set("note", raw).await.unwrap();
    let store = Store::<Note>::new(backend.clone())
        .with_schema(schema())
        .with_cache(CacheConfig::new(4, 4096).unwrap());
    let other = store.clone();
    let (first, second) = futures::join!(store.get("note"), other.get("note"));
    let expected = Note {
        title: "old".into(),
        pinned: false,
    };

    assert_eq!(
        first.unwrap(),
        Some(Note {
            title: "old".into(),
            pinned: false
        })
    );
    assert_eq!(second.unwrap(), Some(expected));
    let persisted: Value =
        serde_json::from_slice(&backend.get("note").await.unwrap().unwrap()).unwrap();

    assert_eq!(
        persisted,
        json!({"$rake_schema": 2, "value": {"title": "old", "pinned": false}})
    );
    let independent = Store::<Note>::new(backend.clone())
        .with_schema(schema())
        .with_cache(CacheConfig::new(4, 4096).unwrap());

    assert_eq!(independent.get("note").await.unwrap().unwrap().title, "old");
    other
        .set(
            "note",
            &Note {
                title: "new".into(),
                pinned: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(store.get("note").await.unwrap().unwrap().title, "new");
    independent.invalidate();
    assert_eq!(independent.get("note").await.unwrap().unwrap().title, "new");
    other.remove("note").await.unwrap();
    assert!(store.get("note").await.unwrap().is_none());

    for raw in [
        br#"{ "name": "old" }"#.as_slice(),
        br#"{"$rake_schema":0,"value":{"name":"old"}}"#,
        br#"{"$rake_schema":1,"value":{"title":"old"}}"#,
    ] {
        backend.set("note", raw).await.unwrap();
        store.invalidate();
        assert_eq!(store.get("note").await.unwrap().unwrap().title, "old");
        assert_ne!(backend.get("note").await.unwrap().unwrap(), raw);
    }

    let failing = Schema::new(2)
        .with_migration(0, |mut value| {
            value["changed"] = json!(true);
            Ok(value)
        })
        .unwrap()
        .with_migration(1, |_| Err(StorageError::Capacity))
        .unwrap();
    let failing = Store::<Value>::new(backend.clone()).with_schema(failing);

    backend.set("note", raw).await.unwrap();
    let error = failing.get("note").await.unwrap_err();

    assert!(
        matches!(&error, StorageError::Migration { from: 1, source } if matches!(**source, StorageError::Capacity))
    );
    assert!(error.to_string().contains("from version 1"));
    assert_eq!(backend.get("note").await.unwrap().unwrap(), raw);

    let future = br#"{ "$rake_schema": 3, "value": {"name":"future"} }"#;

    backend.set("note", future).await.unwrap();
    store.invalidate();
    let error = store.get("note").await.unwrap_err();

    assert!(matches!(
        error,
        StorageError::NewerVersion {
            stored: 3,
            current: 2
        }
    ));
    assert!(error.to_string().contains("newer than supported version 2"));
    assert_eq!(backend.get("note").await.unwrap().unwrap(), future);

    backend.set("note", raw).await.unwrap();
    let missing = Store::<Note>::new(backend.clone()).with_schema(Schema::new(2));

    assert!(matches!(
        missing.get("note").await,
        Err(StorageError::MissingMigration { from: 0 })
    ));
    assert_eq!(backend.get("note").await.unwrap().unwrap(), raw);
    let invalid_type = Store::<Note>::new(backend.clone())
        .with_schema(Schema::new(1).with_migration(0, |_| Ok(json!(42))).unwrap());

    assert!(matches!(
        invalid_type.get("note").await,
        Err(StorageError::Codec(_))
    ));
    assert_eq!(backend.get("note").await.unwrap().unwrap(), raw);
    let oversized = Store::<Value>::new(backend.clone()).with_schema(
        Schema::new(1)
            .with_migration(0, |_| Ok(json!("x".repeat(rake::storage::MAX_VALUE_BYTES))))
            .unwrap(),
    );

    assert!(matches!(
        oversized.get("note").await,
        Err(StorageError::ValueTooLarge)
    ));
    assert_eq!(backend.get("note").await.unwrap().unwrap(), raw);

    let unversioned = Store::<Value>::new(backend.clone());
    let value = json!({"name":"plain"});

    unversioned.set("plain", &value).await.unwrap();
    assert_eq!(
        backend.get("plain").await.unwrap().unwrap(),
        serde_json::to_vec(&value).unwrap()
    );
    assert_eq!(unversioned.get("plain").await.unwrap(), Some(value));
}

#[test]
fn memory_migration_contract() {
    block_on(backend_contract(MemoryBackend::new()));
}

#[cfg(not(target_family = "wasm"))]
#[test]
fn file_migration_contract_and_failed_reads_preserve_the_whole_file() {
    block_on(async {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("private");
        let backend = rake::storage::FileBackend::open(&path).await.unwrap();

        backend_contract(backend.clone()).await;
        let before = std::fs::read(path.join("store.json")).unwrap();
        let failing = Store::<Value>::new(backend.clone()).with_schema(
            Schema::new(1)
                .with_migration(0, |_| Err(StorageError::Capacity))
                .unwrap(),
        );

        assert!(failing.get("note").await.is_err());
        assert_eq!(std::fs::read(path.join("store.json")).unwrap(), before);
        let future = Schema::new(9).encode(&json!({"title":"future"})).unwrap();

        backend.set("note", &future).await.unwrap();
        let before = std::fs::read(path.join("store.json")).unwrap();

        assert!(matches!(
            failing.get("note").await,
            Err(StorageError::NewerVersion { .. })
        ));
        assert_eq!(std::fs::read(path.join("store.json")).unwrap(), before);
        let reopened = Store::<Value>::new(rake::storage::FileBackend::open(&path).await.unwrap())
            .with_schema(Schema::new(9));

        assert_eq!(
            reopened.get("note").await.unwrap(),
            Some(json!({"title":"future"}))
        );
    });
}

#[test]
fn fixture_codec_and_registration_validation() {
    let fixture = include_bytes!("fixtures/storage-v0.json");
    let (note, changed) = schema().decode::<Note>(fixture).unwrap();

    assert_eq!(
        note,
        Note {
            title: "fixture 👋".into(),
            pinned: false
        }
    );
    assert!(changed);
    let bytes = schema().encode(&note).unwrap();
    let (decoded, changed) = schema().decode::<Note>(&bytes).unwrap();

    assert_eq!(decoded, note);
    assert!(!changed);
    assert!(Schema::new(0).with_migration(0, Ok).is_err());
    assert!(
        Schema::new(2)
            .with_migration(0, Ok)
            .unwrap()
            .with_migration(0, Ok)
            .is_err()
    );
    assert!(matches!(
        schema().decode::<u8>(fixture),
        Err(StorageError::Codec(_))
    ));
    for malformed in [
        br#"{"$rake_schema":"bad","value":0}"#.as_slice(),
        br#"{"$rake_schema":0}"#,
        br#"{"$rake_schema":0,"value":0,"extra":1}"#,
    ] {
        assert!(matches!(
            schema().decode::<Value>(malformed),
            Err(StorageError::Codec(_))
        ));
    }
    let (value, changed) = Schema::new(0).decode::<Value>(b"null").unwrap();

    assert_eq!(value, Value::Null);
    assert!(changed);
}

#[derive(Clone, Default)]
struct Observed {
    backend: MemoryBackend,
    reads: Rc<Cell<usize>>,
    writes: Rc<Cell<usize>>,
    read_delay: Rc<RefCell<Option<oneshot::Receiver<()>>>>,
    write_delay: Rc<RefCell<Option<oneshot::Receiver<()>>>>,
    fail: Rc<Cell<bool>>,
}

impl Backend for Observed {
    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> LocalBoxFuture<'a, Result<Option<Vec<u8>>, StorageError>> {
        Box::pin(async move {
            self.reads.set(self.reads.get() + 1);
            let bytes = self.backend.get(key).await?;
            let delay = self.read_delay.borrow_mut().take();

            if let Some(delay) = delay {
                delay.await.unwrap();
            }

            Ok(bytes)
        })
    }

    fn set<'a>(
        &'a self,
        key: &'a str,
        value: &'a [u8],
    ) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async move {
            self.writes.set(self.writes.get() + 1);
            let delay = self.write_delay.borrow_mut().take();

            if let Some(delay) = delay {
                delay.await.unwrap();
            }

            if self.fail.get() {
                return Err(StorageError::Capacity);
            }

            self.backend.set(key, value).await
        })
    }

    fn remove<'a>(&'a self, key: &'a str) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        self.backend.remove(key)
    }

    fn keys<'a>(
        &'a self,
        after: Option<&'a str>,
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<String>, StorageError>> {
        self.backend.keys(after, limit)
    }
}

#[test]
fn migrated_cache_and_concurrent_clones_do_not_repeat_steps_or_overwrite_new_writes() {
    block_on(async {
        let backend = Observed::default();

        backend
            .backend
            .set("note", br#"{"name":"old"}"#)
            .await
            .unwrap();
        let store = Store::<Note>::new(backend.clone())
            .with_schema(schema())
            .with_cache(CacheConfig::new(4, 4096).unwrap());
        let other = store.clone();
        let (release, receiver) = oneshot::channel();

        *backend.read_delay.borrow_mut() = Some(receiver);
        let mut read = Box::pin(store.get("note"));
        let mut shared_read = Box::pin(other.get("note"));

        assert!(futures::poll!(&mut read).is_pending());
        assert!(futures::poll!(&mut shared_read).is_pending());
        release.send(()).unwrap();
        assert_eq!(read.await.unwrap().unwrap().title, "old");
        assert_eq!(shared_read.await.unwrap().unwrap().title, "old");
        assert_eq!(
            backend.writes.get(),
            1,
            "one migration write-back across clones"
        );
        let reads = backend.reads.get();

        assert_eq!(store.get("note").await.unwrap().unwrap().title, "old");
        assert_eq!(backend.reads.get(), reads, "current envelope is cached");

        backend
            .backend
            .set("note", br#"{"name":"old again"}"#)
            .await
            .unwrap();
        store.invalidate();
        let (release, receiver) = oneshot::channel();

        *backend.read_delay.borrow_mut() = Some(receiver);
        let mut read = Box::pin(store.get("note"));
        let replacement = Note {
            title: "new".into(),
            pinned: true,
        };
        let mut write = Box::pin(other.set("note", &replacement));

        assert!(futures::poll!(&mut read).is_pending());
        assert!(futures::poll!(&mut write).is_pending());
        release.send(()).unwrap();
        assert_eq!(read.await.unwrap().unwrap().title, "old again");
        write.await.unwrap();
        assert_eq!(store.get("note").await.unwrap().unwrap(), replacement);
    });
}

#[test]
fn failed_and_cancelled_writeback_preserves_bytes_and_releases_the_operation_lock() {
    block_on(async {
        let backend = Observed::default();
        let raw = br#"{"name":"old"}"#;

        backend.backend.set("note", raw).await.unwrap();
        let store = Store::<Note>::new(backend.clone())
            .with_schema(schema())
            .with_cache(CacheConfig::new(4, 4096).unwrap());

        backend.fail.set(true);
        assert!(matches!(
            store.get("note").await,
            Err(StorageError::Capacity)
        ));
        assert_eq!(backend.backend.get("note").await.unwrap().unwrap(), raw);
        backend.fail.set(false);
        let (_release, receiver) = oneshot::channel();

        *backend.write_delay.borrow_mut() = Some(receiver);
        let mut read = Box::pin(store.get("note"));

        assert!(futures::poll!(&mut read).is_pending());
        drop(read);
        assert_eq!(backend.backend.get("note").await.unwrap().unwrap(), raw);
        assert_eq!(
            store.clone().get("note").await.unwrap().unwrap().title,
            "old"
        );
        let reads = backend.reads.get();

        store.get("note").await.unwrap();
        store.get("note").await.unwrap();
        assert_eq!(
            backend.reads.get(),
            reads + 2,
            "cancellation disables caching"
        );
    });
}

#[test]
fn migration_serialization_failure_preserves_the_original() {
    #[derive(Debug, Deserialize)]
    struct Unwritable;

    impl Serialize for Unwritable {
        fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("cannot encode upgraded value"))
        }
    }

    block_on(async {
        let backend = MemoryBackend::new();
        let raw = b"null";

        backend.set("note", raw).await.unwrap();
        let store = Store::<Unwritable>::new(backend.clone())
            .with_schema(Schema::new(1).with_migration(0, Ok).unwrap());
        let error = store.get("note").await.unwrap_err();

        assert!(matches!(error, StorageError::Codec(_)));
        assert!(error.to_string().contains("cannot encode upgraded value"));
        assert_eq!(backend.get("note").await.unwrap().unwrap(), raw);
    });
}

#[test]
fn application_validation_error_is_the_migration_source_and_preserves_bytes() {
    use std::error::Error;

    block_on(async {
        let backend = MemoryBackend::new();
        let raw = b"[]";

        backend.set("note", raw).await.unwrap();
        let schema = Schema::new(1)
            .with_migration(0, |value| {
                value
                    .as_object()
                    .ok_or_else(|| StorageError::invalid_data("expected a legacy note object"))?;

                Ok(value)
            })
            .unwrap();
        let store = Store::<Value>::new(backend.clone()).with_schema(schema);
        let error = store.get("note").await.unwrap_err();

        assert!(matches!(
            &error,
            StorageError::Migration { from: 0, source }
                if matches!(source.as_ref(), StorageError::InvalidData(message)
                    if message == "expected a legacy note object")
        ));
        let source = error
            .source()
            .expect("migration retains the validation error");

        assert_eq!(
            source.to_string(),
            "storage data is invalid: expected a legacy note object"
        );
        assert_eq!(backend.get("note").await.unwrap().unwrap(), raw);
    });
}
