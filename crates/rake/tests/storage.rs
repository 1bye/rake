//! Covers: the public typed storage contract and validation before mutation.
//! Real: typed facade and Serde conversion.
//! Doubles: a local byte backend with observable calls and corrupt data.
//! Requires: none.

use futures::{executor::block_on, future::LocalBoxFuture};
use rake::storage::{Backend, StorageError, Store};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

#[derive(Clone, Default)]
struct Bytes(Rc<RefCell<BTreeMap<String, Vec<u8>>>>);
impl Backend for Bytes {
    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> LocalBoxFuture<'a, Result<Option<Vec<u8>>, StorageError>> {
        Box::pin(async move { Ok(self.0.borrow().get(key).cloned()) })
    }
    fn set<'a>(
        &'a self,
        key: &'a str,
        value: &'a [u8],
    ) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async move {
            self.0.borrow_mut().insert(key.into(), value.into());
            Ok(())
        })
    }
    fn remove<'a>(&'a self, key: &'a str) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async move {
            self.0.borrow_mut().remove(key);
            Ok(())
        })
    }
    fn keys<'a>(
        &'a self,
        after: Option<&'a str>,
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<String>, StorageError>> {
        Box::pin(async move {
            Ok(self
                .0
                .borrow()
                .keys()
                .filter(|k| after.is_none_or(|a| k.as_str() > a))
                .take(limit)
                .cloned()
                .collect())
        })
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Preferences {
    theme: String,
}

#[test]
fn typed_roundtrip_shared_handles_and_failed_conversion() {
    block_on(async {
        let bytes = Bytes::default();
        let store = Store::<Preferences>::new(bytes.clone());
        let shared = store.clone(); // Preferences deliberately does not implement Clone.
        assert_eq!(store.get("settings").await.unwrap(), None);
        let preferences = Preferences {
            theme: "dark".into(),
        };
        store.set("settings", &preferences).await.unwrap();
        assert_eq!(shared.get("settings").await.unwrap(), Some(preferences));
        assert!(matches!(
            store
                .set(
                    "",
                    &Preferences {
                        theme: "light".into()
                    }
                )
                .await,
            Err(StorageError::InvalidKey)
        ));
        assert!(matches!(
            store.keys(None, 0).await,
            Err(StorageError::InvalidPage)
        ));
        bytes
            .0
            .borrow_mut()
            .insert("broken".into(), b"{\"theme\":3}".to_vec());
        assert!(matches!(
            store.get("broken").await,
            Err(StorageError::Codec(_))
        ));
        assert_eq!(bytes.0.borrow()["broken"], b"{\"theme\":3}");
        assert_eq!(store.keys(Some("broken"), 1).await.unwrap(), ["settings"]);
        shared.remove("settings").await.unwrap();
        shared.remove("settings").await.unwrap();
        assert!(store.get("settings").await.unwrap().is_none());
    });
}

#[test]
fn size_bounds_preserve_existing_values() {
    block_on(async {
        let bytes = Bytes::default();
        let store = Store::<String>::new(bytes.clone());
        store.set("key", &"old".into()).await.unwrap();
        let oversized = "x".repeat(rake::storage::MAX_VALUE_BYTES);
        assert!(matches!(
            store.set("key", &oversized).await,
            Err(StorageError::ValueTooLarge)
        ));
        assert_eq!(store.get("key").await.unwrap().as_deref(), Some("old"));
        assert!(matches!(
            store.get(&"é".repeat(129)).await,
            Err(StorageError::InvalidKey)
        ));
        bytes.0.borrow_mut().insert(
            "large".into(),
            vec![b' '; rake::storage::MAX_VALUE_BYTES + 1],
        );
        assert!(matches!(
            store.get("large").await,
            Err(StorageError::ValueTooLarge)
        ));
    });
}
