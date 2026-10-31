//! Covers: cache hits/eviction/budgets, shared invalidation, async races and cancellation.
//! Real: typed store, bounded cache and memory backend.
//! Doubles: backend call counter and manually released I/O futures.
//! Requires: none.

use futures::{channel::oneshot, executor::block_on, future::LocalBoxFuture};
use rake::storage::{Backend, CacheConfig, MemoryBackend, StorageError, Store};

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Clone, Default)]
struct Observed {
    bytes: MemoryBackend,
    reads: Rc<Cell<usize>>,
    delay: Rc<RefCell<Option<oneshot::Receiver<()>>>>,
    delay_write: Rc<Cell<bool>>,
    fail_write: Rc<Cell<bool>>,
}

impl Backend for Observed {
    fn get<'a>(
        &'a self,
        key: &'a str,
    ) -> LocalBoxFuture<'a, Result<Option<Vec<u8>>, StorageError>> {
        Box::pin(async move {
            self.reads.set(self.reads.get() + 1);
            let value = self.bytes.get(key).await?;
            let delay = self.delay.borrow_mut().take();

            if let Some(delay) = delay {
                delay.await.expect("release delayed read");
            }

            Ok(value)
        })
    }

    fn set<'a>(
        &'a self,
        key: &'a str,
        value: &'a [u8],
    ) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async move {
            if self.delay_write.get() {
                futures::future::pending::<()>().await;
            }

            if self.fail_write.get() {
                return Err(StorageError::Capacity);
            }

            self.bytes.set(key, value).await
        })
    }

    fn remove<'a>(&'a self, key: &'a str) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        self.bytes.remove(key)
    }

    fn keys<'a>(
        &'a self,
        after: Option<&'a str>,
        limit: usize,
    ) -> LocalBoxFuture<'a, Result<Vec<String>, StorageError>> {
        self.bytes.keys(after, limit)
    }
}

#[test]
fn hits_lru_byte_budgets_and_shared_invalidation() {
    block_on(async {
        assert!(CacheConfig::new(0, 10).is_err());
        let backend = Observed::default();

        let store =
            Store::<String>::new(backend.clone()).with_cache(CacheConfig::new(2, 100).unwrap());

        for key in ["a", "b", "c"] {
            store.set(key, &key.to_owned()).await.unwrap();
        }

        for key in ["a", "b", "a", "c", "a"] {
            store.get(key).await.unwrap();
        }

        assert_eq!(backend.reads.get(), 3, "recently read a survives eviction");
        store.get("b").await.unwrap();
        assert_eq!(backend.reads.get(), 4, "b was evicted");
        let shared = store.clone();

        shared.set("a", &"new".into()).await.unwrap();
        assert_eq!(store.get("a").await.unwrap().as_deref(), Some("new"));
        backend.fail_write.set(true);
        assert!(shared.set("a", &"failed".into()).await.is_err());
        assert_eq!(store.get("a").await.unwrap().as_deref(), Some("new"));
        backend.fail_write.set(false);
        let independent = Store::<String>::new(backend.bytes.clone());

        independent.set("a", &"outside".into()).await.unwrap();
        shared.invalidate();
        assert_eq!(store.get("a").await.unwrap().as_deref(), Some("outside"));
        shared.remove("a").await.unwrap();
        assert!(store.get("a").await.unwrap().is_none());
        let before = backend.reads.get();

        assert!(store.get("missing").await.unwrap().is_none());
        assert!(shared.get("missing").await.unwrap().is_none());
        assert_eq!(backend.reads.get(), before + 1, "absence is cached too");

        let tiny =
            Store::<String>::new(backend.clone()).with_cache(CacheConfig::new(2, 1).unwrap());

        let before = backend.reads.get();

        tiny.get("b").await.unwrap();
        tiny.get("b").await.unwrap();
        assert_eq!(
            backend.reads.get(),
            before + 2,
            "large entries bypass cache"
        );
    });
}

#[test]
fn delayed_old_read_cannot_repopulate_after_write() {
    block_on(async {
        let backend = Observed::default();

        let store =
            Store::<String>::new(backend.clone()).with_cache(CacheConfig::new(2, 100).unwrap());

        store.set("a", &"old".into()).await.unwrap();
        let (release, receiver) = oneshot::channel();

        *backend.delay.borrow_mut() = Some(receiver);
        let mut read = Box::pin(store.get("a"));

        assert!(futures::poll!(&mut read).is_pending());
        store.set("a", &"new".into()).await.unwrap();
        release.send(()).unwrap();
        assert_eq!(read.await.unwrap().as_deref(), Some("old"));
        assert_eq!(store.get("a").await.unwrap().as_deref(), Some("new"));
        assert_eq!(backend.reads.get(), 2);
    });
}

#[test]
fn cancelled_write_disables_cache_across_handles() {
    block_on(async {
        let backend = Observed::default();

        let store =
            Store::<String>::new(backend.clone()).with_cache(CacheConfig::new(2, 100).unwrap());

        store.set("a", &"old".into()).await.unwrap();
        store.get("a").await.unwrap();
        backend.delay_write.set(true);
        let value = "new".into();
        let mut write = Box::pin(store.set("a", &value));

        assert!(futures::poll!(&mut write).is_pending());
        drop(write);
        let before = backend.reads.get();

        store.clone().get("a").await.unwrap();
        store.get("a").await.unwrap();
        assert_eq!(
            backend.reads.get(),
            before + 2,
            "cancelled backend work may still commit"
        );
    });
}
