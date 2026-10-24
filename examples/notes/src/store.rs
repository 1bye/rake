//! Shared notebook service: edits, recovery checkpoints and debounced durable saves.

use crate::{Library, Note, Repository, StorageError};
use event_listener::Event;
use futures::future::{Either, select};
use futures::lock::Mutex;
use rake::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::Duration;

struct State {
    library: Option<Library>,
    revision: u64,
    durable: u64,
    checkpoint: u64,
    sequence: u64,
    notice: Option<String>,
    error: Option<StorageError>,
    loading: bool,
    opened: bool,
    pending_restore: bool,
}

#[cfg(feature = "gui")]
type Observer = Rc<dyn Fn(bool)>;
type WeakObserver = Weak<dyn Fn(bool)>;

struct Core {
    dirty_stamp: Cell<(u64, bool)>,
    observers: RefCell<Vec<WeakObserver>>,
    state: RefCell<State>,
    event: Event,
    repository: Result<Repository, StorageError>,
    writer: Mutex<()>,
    delay: Duration,
}

/// One shared notebook, composed once with `App::service` and resolved by views/commands.
#[derive(Clone)]
pub struct Notebook(Rc<Core>);

/// Compact typed command result; full bodies stay out of list queries.
#[rake::data(schema)]
#[derive(Clone, Debug)]
pub struct NoteSummary {
    /// Stable document key.
    pub id: u64,
    /// User-facing title.
    pub title: String,
    /// Optimistic edit version.
    pub version: u64,
    /// Current word count.
    pub words: usize,
    /// Pinned collection membership.
    pub pinned: bool,
    /// Soft deletion state.
    pub archived: bool,
}

impl From<&Note> for NoteSummary {
    fn from(note: &Note) -> Self {
        Self {
            id: note.id,
            title: note.display_title().into(),
            version: note.version,
            words: note.word_count(),
            pinned: note.pinned,
            archived: note.archived,
        }
    }
}

/// Create a note through the same service used by the editor.
#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "notes.create", output = NoteSummary, capability = Headless)]
#[serde(deny_unknown_fields)]
pub struct CreateNote {
    /// Initial title.
    #[serde(default)]
    pub title: String,
    /// Initial plain text.
    #[serde(default)]
    pub body: String,
}

/// Edit only an explicitly observed document version.
#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "notes.edit", output = NoteSummary, capability = Headless)]
#[serde(deny_unknown_fields)]
pub struct EditNote {
    /// Concrete document.
    pub id: u64,
    /// Version returned by notes.list or the preceding edit.
    pub expected_version: u64,
    /// Replacement title; None preserves it.
    pub title: Option<String>,
    /// Replacement body; None preserves it.
    pub body: Option<String>,
}

/// List current documents and versions without retaining a window.
#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "notes.list", output = Vec<NoteSummary>, capability = Headless)]
pub struct ListNotes;

/// Flush all current notes, or just their recovery checkpoint.
#[rake::data(schema)]
#[derive(rake::Command)]
#[command(id = "notes.save", output = u64, capability = Headless)]
#[serde(deny_unknown_fields)]
pub struct SaveNotes {
    /// Development/recovery inspection; normal saves leave this false.
    #[serde(default)]
    pub checkpoint_only: bool,
}

impl Notebook {
    /// Creates an unloaded notebook with a 700 ms quiet-period autosave.
    pub fn new(repository: Repository) -> Self {
        Self::with_autosave_delay(repository, Duration::from_millis(700))
    }

    /// Configures the quiet period. Recovery checkpoints still happen immediately;
    /// continuous typing triggers a main-file save after the larger of two seconds
    /// and the configured quiet period.
    pub fn with_autosave_delay(repository: Repository, delay: Duration) -> Self {
        Self::configured(Ok(repository), delay)
    }

    pub(crate) fn configured(
        repository: Result<Repository, StorageError>,
        delay: Duration,
    ) -> Self {
        Self(Rc::new(Core {
            dirty_stamp: Cell::new((0, false)),
            observers: RefCell::new(Vec::new()),
            state: RefCell::new(State {
                library: None,
                revision: 0,
                durable: 0,
                checkpoint: 0,
                sequence: 0,
                notice: None,
                error: None,
                loading: true,
                opened: false,
                pending_restore: false,
            }),
            event: Event::new(),
            repository,
            writer: Mutex::new(()),
            delay,
        }))
    }

    fn notify(&self) {
        self.0.state.borrow_mut().sequence += 1;
        self.0.event.notify(usize::MAX);
        let observers: Vec<_> = self
            .0
            .observers
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .collect();

        self.0
            .observers
            .borrow_mut()
            .retain(|observer| observer.strong_count() > 0);
        let dirty = self.is_dirty();
        let stamp = (self.revision(), dirty);

        if self.0.dirty_stamp.replace(stamp) != stamp {
            for observer in observers {
                observer(dirty);
            }
        }
    }

    #[cfg(feature = "gui")]
    pub(crate) fn observe_dirty(&self, observer: impl Fn(bool) + 'static) -> Observer {
        let observer: Observer = Rc::new(observer);

        self.0.observers.borrow_mut().push(Rc::downgrade(&observer));
        observer(self.is_dirty());

        observer
    }

    /// Loads from durable state. Failures preserve stored files and are visible to the UI.
    pub async fn load(&self) -> Result<(), StorageError> {
        let _writer = self.0.writer.lock().await;

        if self.0.state.borrow().opened {
            return Ok(());
        }

        let result = match &self.0.repository {
            Ok(repository) => repository.load().await,
            Err(error) => Err(error.clone()),
        };

        let mut state = self.0.state.borrow_mut();

        state.loading = false;

        match result {
            Ok(report) => {
                if state.pending_restore
                    && state.revision == report.revision
                    && state.library.as_ref() != Some(&report.library)
                {
                    state.error = Some(StorageError::Conflict);
                    drop(state);
                    self.notify();

                    return Err(StorageError::Conflict);
                }

                if !state.pending_restore || report.revision > state.revision {
                    state.revision = report.revision + u64::from(report.needs_save);
                    state.library = Some(report.library);
                }

                state.durable = report.revision;
                state.checkpoint = report.revision;
                state.notice = report.notice;
                state.error = None;
                state.opened = true;
                state.pending_restore = false;
            }
            Err(error) => {
                state.error = Some(error.clone());
                drop(state);
                self.notify();

                return Err(error);
            }
        }

        drop(state);
        self.notify();

        Ok(())
    }

    #[cfg(feature = "gui")]
    pub(crate) fn restore_pending(
        &self,
        library: Library,
        revision: u64,
    ) -> Result<(), StorageError> {
        library.validate()?;
        let mut state = self.0.state.borrow_mut();

        if revision == 0 || revision > 9_007_199_254_740_990 || state.opened {
            return Err(StorageError::Invalid(
                "Cannot restore a notebook into an already opened library.".into(),
            ));
        }

        state.library = Some(library);
        state.revision = revision;
        state.pending_restore = true;
        drop(state);
        self.notify();

        Ok(())
    }

    /// Owned data for presentation; unavailable until loading succeeds.
    pub fn library(&self) -> Option<Library> {
        self.0.state.borrow().library.clone()
    }

    /// Whether the latest edit has reached the normal saved snapshot.
    pub fn is_dirty(&self) -> bool {
        let state = self.0.state.borrow();

        state.library.is_some() && state.revision != state.durable
    }

    /// Latest model/diagnostic change, used for gap-free local observation.
    pub fn sequence(&self) -> u64 {
        self.0.state.borrow().sequence
    }

    /// Current content revision.
    pub fn revision(&self) -> u64 {
        self.0.state.borrow().revision
    }

    /// Last load/save error, without inventing a successful save.
    pub fn error(&self) -> Option<String> {
        self.0
            .state
            .borrow()
            .error
            .as_ref()
            .map(ToString::to_string)
    }

    /// Recovery/migration notice.
    pub fn notice(&self) -> Option<String> {
        self.0.state.borrow().notice.clone()
    }

    /// Startup is still in flight.
    pub fn is_loading(&self) -> bool {
        self.0.state.borrow().loading
    }

    /// Whether all current edits have a durable recovery checkpoint.
    pub fn is_checkpointed(&self) -> bool {
        let state = self.0.state.borrow();

        state.revision == state.checkpoint
    }

    /// Clears a non-destructive recovery notice.
    pub fn dismiss_notice(&self) {
        self.0.state.borrow_mut().notice = None;
        self.notify();
    }

    /// Waits without losing changes between a snapshot and subscription.
    pub async fn changed(&self, sequence: u64) {
        loop {
            let listener = self.0.event.listen();

            if self.sequence() != sequence {
                return;
            }

            listener.await;
        }
    }

    fn change<T>(
        &self,
        apply: impl FnOnce(&mut Library) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let mut state = self.0.state.borrow_mut();

        if matches!(state.error, Some(StorageError::Conflict)) {
            return Err(StorageError::Conflict);
        }

        let mut library = state
            .library
            .clone()
            .ok_or_else(|| StorageError::Invalid("Library is not ready.".into()))?;

        let result = apply(&mut library)?;

        library.validate()?;

        if state.revision >= 9_007_199_254_740_990 {
            return Err(StorageError::Invalid("Library revision exhausted.".into()));
        }

        state.library = Some(library);
        state.revision += 1;
        state.error = None;
        drop(state);
        self.notify();

        Ok(result)
    }

    /// Creates and selects a document. No existing document is overwritten.
    pub fn create(&self, title: String, body: String) -> Result<NoteSummary, StorageError> {
        self.change(|library| {
            let note = Note {
                id: library.next_id,
                title,
                body,
                pinned: false,
                archived: false,
                modified_ms: crate::model::now_ms(),
                version: 1,
            };

            library.next_id = library
                .next_id
                .checked_add(1)
                .ok_or_else(|| StorageError::Invalid("Document IDs exhausted.".into()))?;
            library.selected = Some(note.id);
            let summary = NoteSummary::from(&note);

            library.notes.push(note);

            Ok(summary)
        })
    }

    /// Optimistic field edits; stale commands never overwrite later UI changes.
    pub fn edit(&self, command: EditNote) -> Result<NoteSummary, StorageError> {
        self.change(|library| {
            let note = library
                .notes
                .iter_mut()
                .find(|note| note.id == command.id)
                .ok_or_else(|| StorageError::Invalid("Note no longer exists.".into()))?;

            if note.version != command.expected_version {
                return Err(StorageError::Conflict);
            }

            if let Some(title) = command.title {
                note.title = title;
            }

            if let Some(body) = command.body {
                note.body = body;
            }

            note.version += 1;
            note.modified_ms = crate::model::now_ms();

            Ok(NoteSummary::from(&*note))
        })
    }

    /// Selects an existing note; selection is restored after restart.
    pub fn select(&self, id: u64) -> Result<(), StorageError> {
        if self
            .library()
            .is_some_and(|library| library.selected == Some(id))
        {
            return Ok(());
        }

        self.change(|library| {
            if !library.notes.iter().any(|note| note.id == id) {
                return Err(StorageError::Invalid("Note no longer exists.".into()));
            }

            library.selected = Some(id);

            Ok(())
        })
    }

    /// Pins/unpins a note without changing its body.
    pub fn pin(&self, id: u64) -> Result<(), StorageError> {
        self.metadata(id, false)
    }

    /// Moves a note to/from Archive; this never permanently deletes its contents.
    pub fn archive(&self, id: u64) -> Result<(), StorageError> {
        self.metadata(id, true)
    }

    fn metadata(&self, id: u64, archive: bool) -> Result<(), StorageError> {
        self.change(|library| {
            let note = library
                .notes
                .iter_mut()
                .find(|note| note.id == id)
                .ok_or_else(|| StorageError::Invalid("Note no longer exists.".into()))?;

            if archive {
                note.archived = !note.archived;
            } else {
                note.pinned = !note.pinned;
            }

            note.version += 1;
            note.modified_ms = crate::model::now_ms();

            Ok(())
        })
    }

    /// Serializes writes; acknowledgements only clear the revision actually written.
    /// A checkpoint precedes main-file replacement so interrupted saves can recover.
    pub async fn save(&self, checkpoint_only: bool) -> Result<u64, StorageError> {
        let _writer = self.0.writer.lock().await;

        let (library, revision, expected) = {
            let state = self.0.state.borrow();

            if !state.opened {
                return Err(state
                    .error
                    .clone()
                    .unwrap_or_else(|| StorageError::Invalid("Library is not ready.".into())));
            }

            (
                state
                    .library
                    .clone()
                    .ok_or_else(|| StorageError::Invalid("Library is not ready.".into()))?,
                state.revision,
                state.checkpoint,
            )
        };

        let result = self
            .0
            .repository
            .as_ref()
            .map_err(Clone::clone)?
            .write(library, revision, expected, checkpoint_only)
            .await;

        let mut state = self.0.state.borrow_mut();

        match result {
            Ok(()) => {
                state.checkpoint = revision;

                if !checkpoint_only {
                    state.durable = revision;
                }

                state.error = None;
            }
            Err(error) => {
                state.error = Some(error.clone());
                drop(state);
                self.notify();

                return Err(error);
            }
        }

        drop(state);
        self.notify();

        Ok(revision)
    }

    /// Flushes the latest revision, including edits made while an earlier save was running.
    pub async fn flush(&self) -> Result<u64, StorageError> {
        while self.is_dirty() {
            self.save(false).await?;
        }

        Ok(self.revision())
    }

    /// Managed activity: checkpoint promptly, coalesce main writes, and retry only
    /// after an edit or explicit save following an error. No hot failure loop.
    pub async fn autosave(&self) {
        loop {
            if !self.is_dirty() || self.error().is_some() || !self.0.state.borrow().opened {
                let sequence = self.sequence();

                self.changed(sequence).await;
                continue;
            }

            if self.save(true).await.is_err() {
                continue;
            }

            let maximum = rake::tasks::sleep(self.0.delay.max(Duration::from_secs(2)));

            futures::pin_mut!(maximum);

            loop {
                let sequence = self.sequence();
                let changed = self.changed(sequence);
                let quiet = rake::tasks::sleep(self.0.delay);
                let next = select(Box::pin(changed), Box::pin(quiet));

                match select(&mut maximum, Box::pin(next)).await {
                    Either::Left(_) => break,
                    Either::Right((Either::Right(_), _)) => break,
                    Either::Right((Either::Left(_), _)) => {
                        if self.save(true).await.is_err() {
                            break;
                        }
                    }
                }
            }

            if self.error().is_none() {
                let _ = self.save(false).await;
            }
        }
    }

    /// Registers typed GUI/CLI handlers that resolve their service from the runtime context.
    pub fn commands() -> Result<Commands, CommandError> {
        let commands = Commands::new();

        Self::register_commands(&commands)?;

        Ok(commands)
    }

    /// Adds the notebook commands to an existing GUI or headless host registry.
    pub fn register_commands(commands: &Commands) -> Result<(), CommandError> {
        fn service(context: &CommandContext) -> Result<Rc<Notebook>, CommandError> {
            context
                .services
                .get()
                .map_err(|error| CommandError::Failed(error.to_string()))
        }

        fn failed(error: StorageError) -> CommandError {
            CommandError::Failed(error.to_string())
        }
        commands.register::<CreateNote, _, _>(|args, context| async move {
            service(&context)?
                .create(args.title, args.body)
                .map_err(failed)
        })?;
        commands.register::<EditNote, _, _>(|args, context| async move {
            service(&context)?.edit(args).map_err(failed)
        })?;
        commands.register::<ListNotes, _, _>(|_, context| async move {
            Ok(service(&context)?
                .library()
                .ok_or_else(|| CommandError::Failed("Library is not ready.".into()))?
                .notes
                .iter()
                .map(NoteSummary::from)
                .collect())
        })?;
        commands.register::<SaveNotes, _, _>(|args, context| async move {
            service(&context)?
                .save(args.checkpoint_only)
                .await
                .map_err(failed)
        })?;

        Ok(())
    }
}

#[cfg(all(test, feature = "gui", not(target_family = "wasm")))]
mod tests {
    //! Covers: reload staging before disk ownership and conflict-safe restoration.
    //! Real: notebook, native repository and files. Doubles: none. Requires: no GUI.
    use super::{Notebook, Repository, StorageError};
    use futures::executor::block_on;

    struct Directory(std::path::PathBuf);

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn reload_data_waits_for_disk_validation_and_then_saves_the_newer_edit() {
        let directory = Directory(
            std::env::temp_dir().join(format!("fieldnotes-reload-{}", uuid::Uuid::new_v4())),
        );

        let notebook = Notebook::new(Repository::at(&directory.0));

        block_on(notebook.load()).unwrap();
        block_on(notebook.flush()).unwrap();
        let revision = notebook.revision();
        let mut library = notebook.library().unwrap();

        library.notes[0].body = "Unsaved reload state".into();
        library.notes[0].version += 1;
        drop(notebook);
        let notebook = Notebook::new(Repository::at(&directory.0));

        notebook.restore_pending(library, revision + 1).unwrap();
        assert!(block_on(notebook.save(true)).is_err());
        block_on(notebook.load()).unwrap();
        assert!(notebook.is_dirty());
        block_on(notebook.flush()).unwrap();
        drop(notebook);
        let notebook = Notebook::new(Repository::at(&directory.0));

        block_on(notebook.load()).unwrap();
        assert_eq!(
            notebook.library().unwrap().notes[0].body,
            "Unsaved reload state"
        );
    }

    #[test]
    fn conflicting_reload_never_overwrites_a_committed_revision() {
        let directory = Directory(
            std::env::temp_dir().join(format!("fieldnotes-reload-{}", uuid::Uuid::new_v4())),
        );

        let notebook = Notebook::new(Repository::at(&directory.0));

        block_on(notebook.load()).unwrap();
        block_on(notebook.flush()).unwrap();
        let revision = notebook.revision();
        let mut library = notebook.library().unwrap();

        library.notes[0].body = "Different text at the same revision".into();
        drop(notebook);
        let before = std::fs::read(directory.0.join("library.json")).unwrap();
        let notebook = Notebook::new(Repository::at(&directory.0));

        notebook.restore_pending(library, revision).unwrap();
        assert_eq!(block_on(notebook.load()), Err(StorageError::Conflict));
        assert_eq!(block_on(notebook.save(false)), Err(StorageError::Conflict));
        assert_eq!(
            std::fs::read(directory.0.join("library.json")).unwrap(),
            before
        );
    }
}
