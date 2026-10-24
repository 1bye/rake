//! Covers: real durable writes, recovery, migration, file safety and autosave semantics.
//! Real: native files, locks, blocking worker, notebook and typed runtime commands.
//! Doubles: none. Requires: native filesystem; no display/network.

#![cfg(not(target_family = "wasm"))]

use futures::FutureExt;
use futures::executor::block_on;
use rake_notes::{CreateNote, EditNote, Notebook, Repository, StorageError};
use std::path::{Path, PathBuf};

struct Directory(PathBuf);

impl Directory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("fieldnotes-test-{}", uuid::Uuid::new_v4())))
    }

    fn notebook(&self) -> Notebook {
        Notebook::new(Repository::at(&self.0))
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn private_write(path: &Path, bytes: impl AsRef<[u8]>) {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();

    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .unwrap()
        .write_all(bytes.as_ref())
        .unwrap();
}

#[test]
fn checkpoint_recovers_newer_text_and_selection_after_an_interrupted_main_save() {
    let directory = Directory::new();
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    block_on(notebook.flush()).unwrap();
    let created = notebook
        .create(
            "日本語のノート".into(),
            "A thought 👩🏽‍💻\nwith another line".into(),
        )
        .unwrap();

    block_on(notebook.save(true)).unwrap();
    drop(notebook);
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    let library = notebook.library().unwrap();

    assert_eq!(library.selected, Some(created.id));
    assert_eq!(
        library.notes.last().unwrap().body,
        "A thought 👩🏽‍💻\nwith another line"
    );
    assert!(notebook.notice().unwrap().contains("Recovered"));
    block_on(notebook.flush()).unwrap();
    drop(notebook);
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    assert!(!notebook.is_dirty());
    assert!(notebook.notice().is_none());
}

#[test]
fn corrupt_main_recovers_a_valid_checkpoint_and_preserves_damaged_bytes() {
    let directory = Directory::new();
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    block_on(notebook.flush()).unwrap();
    drop(notebook);
    private_write(&directory.0.join("library.json"), b"{interrupted");
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    assert_eq!(notebook.library().unwrap().notes.len(), 3);
    assert!(notebook.notice().unwrap().contains("Recovered"));
    block_on(notebook.flush()).unwrap();
    assert!(std::fs::read_dir(&directory.0).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("damaged-")
    }));
}

#[test]
fn previous_copy_can_repair_both_broken_current_snapshots() {
    let directory = Directory::new();
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    block_on(notebook.flush()).unwrap();
    notebook
        .create("A new note".into(), "Newer text".into())
        .unwrap();
    block_on(notebook.flush()).unwrap();
    drop(notebook);
    private_write(&directory.0.join("library.json"), b"{broken");
    private_write(&directory.0.join("recovery.json"), b"{broken");
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    assert_eq!(notebook.library().unwrap().notes.len(), 3);
    assert!(notebook.notice().is_some());
    block_on(notebook.flush()).unwrap();
    drop(notebook);
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    assert_eq!(notebook.library().unwrap().notes.len(), 3);
    assert!(!notebook.is_dirty());
}

#[test]
fn malformed_higher_revision_checkpoint_does_not_block_repair_from_valid_main() {
    let directory = Directory::new();
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    block_on(notebook.flush()).unwrap();
    drop(notebook);
    let saved = std::fs::read(directory.0.join("library.json")).unwrap();
    let mut damaged: rake::serde_json::Value = rake::serde_json::from_slice(&saved).unwrap();

    damaged["revision"] = rake::serde_json::json!(1000);
    damaged["library"]["notes"][0]["id"] = rake::serde_json::json!(0);
    private_write(
        &directory.0.join("recovery.json"),
        rake::serde_json::to_vec(&damaged).unwrap(),
    );
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    assert!(notebook.notice().is_some());
    assert_eq!(notebook.library().unwrap().notes[0].id, 1);
    block_on(notebook.flush()).unwrap();
    drop(notebook);
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    assert!(!notebook.is_dirty());
    assert!(notebook.notice().is_none());
}

#[test]
fn unknown_formats_and_total_corruption_never_create_a_replacement_library() {
    let directory = Directory::new();
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    drop(notebook);
    let raw = r#"{"format":99,"revision":5}"#;

    private_write(&directory.0.join("library.json"), raw);
    let notebook = directory.notebook();

    assert_eq!(
        block_on(notebook.load()).unwrap_err(),
        StorageError::Unsupported(99)
    );
    assert!(notebook.library().is_none());
    assert!(notebook.create("lost?".into(), String::new()).is_err());
    assert_eq!(
        std::fs::read_to_string(directory.0.join("library.json")).unwrap(),
        raw
    );
    drop(notebook);
    private_write(&directory.0.join("library.json"), b"{broken");
    let notebook = directory.notebook();

    assert!(block_on(notebook.load()).is_err());
    assert!(notebook.library().is_none());
}

#[test]
fn legacy_data_migrates_and_keeps_its_original_saved_copy() {
    let directory = Directory::new();
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    drop(notebook);
    private_write(
        &directory.0.join("library.json"),
        r#"{"format":0,"revision":7,"selected":42,"notes":[{"id":42,"title":"Old note","text":"Still here"}]}"#,
    );
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    assert_eq!(notebook.library().unwrap().notes[0].body, "Still here");
    assert!(notebook.notice().unwrap().contains("upgraded"));
    block_on(notebook.flush()).unwrap();
    let old: rake::serde_json::Value =
        rake::serde_json::from_slice(&std::fs::read(directory.0.join("previous.json")).unwrap())
            .unwrap();

    let new: rake::serde_json::Value =
        rake::serde_json::from_slice(&std::fs::read(directory.0.join("library.json")).unwrap())
            .unwrap();

    assert_eq!(old["format"], 0);
    assert_eq!(new["format"], 1);
    assert_eq!(new["library"]["selected"], 42);
}

#[test]
fn a_second_native_instance_cannot_write_until_the_first_releases_its_lease() {
    let directory = Directory::new();
    let first = directory.notebook();
    let second = directory.notebook();

    block_on(first.load()).unwrap();
    assert_eq!(block_on(second.load()).unwrap_err(), StorageError::Locked);
    drop(first);
    block_on(second.load()).unwrap();
    assert!(second.library().is_some());
}

#[test]
fn stale_or_oversized_edits_leave_the_previous_document_intact() {
    let directory = Directory::new();
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    let created = notebook.create("draft".into(), "one".into()).unwrap();

    notebook
        .edit(EditNote {
            id: created.id,
            expected_version: 1,
            title: None,
            body: Some("two".into()),
        })
        .unwrap();
    let before = notebook.library().unwrap();

    assert_eq!(
        notebook
            .edit(EditNote {
                id: created.id,
                expected_version: 1,
                title: None,
                body: Some("stale".into())
            })
            .unwrap_err(),
        StorageError::Conflict
    );
    assert!(
        notebook
            .edit(EditNote {
                id: created.id,
                expected_version: 2,
                title: None,
                body: Some("x".repeat(512 * 1024 + 1))
            })
            .is_err()
    );
    assert_eq!(notebook.library().unwrap(), before);
}

#[test]
fn acknowledgement_does_not_mark_edits_made_during_a_save_as_already_saved() {
    let directory = Directory::new();
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    let created = notebook.create("draft".into(), "one".into()).unwrap();
    let mut save = Box::pin(notebook.save(false));
    let early = save.as_mut().now_or_never();

    notebook
        .edit(EditNote {
            id: created.id,
            expected_version: 1,
            title: None,
            body: Some("newer".into()),
        })
        .unwrap();

    match early {
        Some(result) => {
            result.unwrap();
        }
        None => {
            block_on(save).unwrap();
        }
    }

    assert!(notebook.is_dirty());
    block_on(notebook.flush()).unwrap();
    assert!(!notebook.is_dirty());
}

#[test]
fn failed_save_retains_dirty_data_and_can_be_retried_without_losing_the_main_file() {
    let directory = Directory::new();
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    block_on(notebook.flush()).unwrap();
    let saved = std::fs::read(directory.0.join("library.json")).unwrap();

    notebook
        .create("unsaved".into(), "keep this".into())
        .unwrap();
    std::fs::remove_file(directory.0.join("recovery.json")).unwrap();
    std::fs::create_dir(directory.0.join("recovery.json")).unwrap();
    assert!(block_on(notebook.save(false)).is_err());
    assert!(notebook.is_dirty());
    assert!(notebook.error().is_some());
    assert_eq!(
        std::fs::read(directory.0.join("library.json")).unwrap(),
        saved
    );
    std::fs::remove_dir(directory.0.join("recovery.json")).unwrap();
    block_on(notebook.flush()).unwrap();
    assert!(!notebook.is_dirty());
}

#[test]
fn autosave_checkpoints_then_commits_without_an_explicit_save_command() {
    let directory = Directory::new();

    let notebook = Notebook::with_autosave_delay(
        Repository::at(&directory.0),
        std::time::Duration::from_millis(20),
    );

    block_on(notebook.load()).unwrap();
    notebook
        .create("automatic".into(), "persist me".into())
        .unwrap();
    block_on(async {
        let wait = async {
            while notebook.is_dirty() {
                let sequence = notebook.sequence();

                notebook.changed(sequence).await;
            }
        };

        let deadline = rake::tasks::sleep(std::time::Duration::from_secs(3));
        let finished = futures::future::select(Box::pin(wait), Box::pin(deadline));

        let result =
            futures::future::select(Box::pin(notebook.autosave()), Box::pin(finished)).await;

        assert!(matches!(
            result,
            futures::future::Either::Right((futures::future::Either::Left(_), _))
        ));
    });
    drop(notebook);
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    assert!(
        notebook
            .library()
            .unwrap()
            .notes
            .iter()
            .any(|note| note.body == "persist me")
    );
}

#[cfg(unix)]
#[test]
fn symlinked_data_is_rejected_without_touching_its_target() {
    let directory = Directory::new();
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    drop(notebook);
    let outside = directory.0.join("outside.json");

    private_write(&outside, b"keep");
    std::os::unix::fs::symlink(&outside, directory.0.join("library.json")).unwrap();
    let notebook = directory.notebook();

    assert!(block_on(notebook.load()).is_err());
    assert_eq!(std::fs::read(outside).unwrap(), b"keep");
}

#[test]
fn typed_commands_resolve_the_application_notebook_and_return_real_document_versions() {
    let directory = Directory::new();
    let notebook = directory.notebook();

    block_on(notebook.load()).unwrap();
    let registry = rake::ServiceRegistry::new().with(notebook.clone()).unwrap();

    let router =
        rake::router::Router::<rake::router::NoRoute>::with_services(Default::default(), registry);

    Notebook::register_commands(&router.client().commands()).unwrap();
    let mut pool = futures::executor::LocalPool::new();

    use futures::task::LocalSpawnExt;
    let driven = router.clone();

    pool.spawner()
        .spawn_local(async move {
            driven.run().await.unwrap();
        })
        .unwrap();
    let reply = router
        .navigator()
        .dispatch(
            CreateNote {
                title: "GUI".into(),
                body: "same service".into(),
            },
            "create-from-gui".into(),
        )
        .unwrap();

    let rake::control::Reply::Submitted { operation_id } = reply else {
        panic!()
    };

    pool.run_until_stalled();
    let result = router
        .client()
        .request(rake::control::Request {
            protocol: rake::control::PROTOCOL_VERSION,
            runtime_id: router.runtime_id(),
            key: None,
            message: rake::control::Message::Query(rake::control::Query::Operation {
                operation: operation_id,
            }),
        })
        .unwrap();

    let rake::control::Reply::Operation(record) = result else {
        panic!()
    };

    assert_eq!(record.state, rake::router::OperationState::Committed);
    let id = record.result.unwrap()["id"].as_u64().unwrap();

    assert_eq!(notebook.library().unwrap().selected, Some(id));
    assert_eq!(
        registry_value(&router)
            .library()
            .unwrap()
            .notes
            .last()
            .unwrap()
            .body,
        "same service"
    );
    router.shutdown();
}

fn registry_value(router: &rake::router::Router<rake::router::NoRoute>) -> std::rc::Rc<Notebook> {
    router.client().services().get().unwrap()
}
