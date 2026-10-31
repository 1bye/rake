//! Covers: interchangeable backends, real persistence, cooperating concurrent
//! writers, corrupt-file preservation and private Unix path handling.
//! Real: memory backend, worker pool, native files, advisory locks and atomic rename.
//! Doubles: none.
//! Requires: a writable temporary directory; file tests run on native targets.

use futures::executor::block_on;
use rake::storage::{MemoryBackend, Store};

#[test]
fn memory_namespaces_are_shared_only_when_cloned() {
    block_on(async {
        let backend = MemoryBackend::new();
        let first = Store::<String>::new(backend.clone());
        let second = Store::<String>::new(backend);

        first.set("../é", &"value".into()).await.unwrap();
        assert_eq!(second.get("../é").await.unwrap().as_deref(), Some("value"));
        assert!(
            Store::<String>::new(MemoryBackend::new())
                .get("../é")
                .await
                .unwrap()
                .is_none()
        );
    });
}

#[cfg(not(target_family = "wasm"))]
mod native {
    use super::*;
    use rake::storage::{FileBackend, StorageError};
    use std::{fs, path::PathBuf};

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            Self(std::env::temp_dir().join(format!("rake-storage-{}", uuid::Uuid::new_v4())))
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn file_reopen_concurrent_writes_and_corruption() {
        block_on(async {
            let directory = Directory::new();
            let first = Store::<String>::new(FileBackend::open(&directory.0).await.unwrap());
            let second = Store::<String>::new(FileBackend::open(&directory.0).await.unwrap());
            let a = "dark".into();
            let b = "日本語".into();
            let (a, b) = futures::join!(first.set("theme", &a), second.set("../language", &b));

            a.unwrap();
            b.unwrap();
            let reopened = Store::<String>::new(FileBackend::open(&directory.0).await.unwrap());

            assert_eq!(
                reopened.get("theme").await.unwrap().as_deref(),
                Some("dark")
            );
            assert_eq!(
                reopened.get("../language").await.unwrap().as_deref(),
                Some("日本語")
            );
            assert_eq!(reopened.keys(None, 1).await.unwrap(), ["../language"]);
            assert_eq!(
                reopened.keys(Some("../language"), 1).await.unwrap(),
                ["theme"]
            );
            reopened.remove("theme").await.unwrap();
            assert!(first.get("theme").await.unwrap().is_none());

            let path = directory.0.join("store.json");

            fs::write(&path, b"corrupt data").unwrap();
            assert!(matches!(
                first.get("theme").await,
                Err(StorageError::Codec(_))
            ));
            assert!(first.set("theme", &"light".into()).await.is_err());
            assert_eq!(fs::read(&path).unwrap(), b"corrupt data");
            assert_eq!(
                fs::read_dir(&directory.0).unwrap().count(),
                2,
                "temporary files cleaned"
            );
        });
    }

    #[cfg(unix)]
    #[test]
    fn file_rejects_public_directories_and_symlink_values() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        block_on(async {
            let directory = Directory::new();

            fs::create_dir(&directory.0).unwrap();
            fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o755)).unwrap();
            assert!(FileBackend::open(&directory.0).await.is_err());
            fs::set_permissions(&directory.0, fs::Permissions::from_mode(0o700)).unwrap();
            let store = Store::<String>::new(FileBackend::open(&directory.0).await.unwrap());
            let outside = directory.0.join("outside");

            fs::write(&outside, b"untouched").unwrap();
            symlink(&outside, directory.0.join("store.json")).unwrap();
            assert!(store.set("key", &"data".into()).await.is_err());
            assert_eq!(fs::read(outside).unwrap(), b"untouched");
        });
    }

    #[test]
    fn full_file_store_rejects_new_keys_without_changing_existing_data() {
        block_on(async {
            let directory = Directory::new();
            let store = Store::<u8>::new(FileBackend::open(&directory.0).await.unwrap());

            store.set("initial", &0).await.unwrap();
            let values: std::collections::BTreeMap<_, _> = (0..4096)
                .map(|index| (format!("key-{index:04}"), vec![b'0']))
                .collect();

            let bytes = serde_json::to_vec(&values).unwrap();
            let path = directory.0.join("store.json");

            fs::write(&path, &bytes).unwrap();
            assert_eq!(store.get("key-0000").await.unwrap(), Some(0));
            assert!(matches!(
                store.set("overflow", &1).await,
                Err(StorageError::Capacity)
            ));
            assert_eq!(fs::read(path).unwrap(), bytes);
        });
    }
}
