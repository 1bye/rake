//! Versioned, bounded document data. UI entities and callbacks never enter storage.

use crate::StorageError;

/// One independently editable plain-text document.
#[rake::data(schema)]
#[derive(Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Note {
    /// Stable local document ID.
    pub id: u64,
    /// Editable title; empty titles display as Untitled note.
    pub title: String,
    /// Plain text with Markdown-friendly formatting.
    pub body: String,
    /// Appears in the pinned collection.
    pub pinned: bool,
    /// Soft deletion; archived documents remain recoverable.
    pub archived: bool,
    /// Last content/metadata edit, milliseconds since the Unix epoch.
    pub modified_ms: u64,
    /// Optimistic edit version, independent of document selection.
    pub version: u64,
}

/// Durable library and selected document. Search/filter state is transient.
#[rake::data]
#[derive(Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Library {
    /// Never reused, including after document deletion.
    pub next_id: u64,
    /// Last selected existing document.
    pub selected: Option<u64>,
    /// Includes archived documents.
    pub notes: Vec<Note>,
}

impl Note {
    /// Human-readable nonempty title.
    pub fn display_title(&self) -> &str {
        if self.title.trim().is_empty() {
            "Untitled note"
        } else {
            self.title.trim()
        }
    }

    /// Unicode-aware whitespace-delimited word count.
    pub fn word_count(&self) -> usize {
        self.body.split_whitespace().count()
    }
}

impl Library {
    pub(crate) fn welcome() -> Self {
        let notes = [
            ("A little space to think", "Welcome to Fieldnotes.\n\nA quiet place for your ideas, working notes, and the things you want to remember.\n\nStart writing here, or create a new note. Your changes are saved automatically on this device.\n\nMAKE IT YOURS\n\n• Give an idea room to grow.\n• Pin the notes you come back to.\n• Find a thought with search.\n\nNo account. No feed. Just your notes.", true),
            ("The next small thing", "Good work starts with a small, clear next step.\n\nToday, I want to…\n\n", false),
            ("A collection of good ideas", "A place to keep the sparks.\n\n— Something worth exploring\n— A sentence worth remembering\n— A question worth asking\n", false),
        ].into_iter().enumerate().map(|(index, (title, body, pinned))| Note {
            id: index as u64 + 1, title: title.into(), body: body.into(), pinned, archived: false,
            modified_ms: now_ms(), version: 1,
        }).collect();

        Self {
            next_id: 4,
            selected: Some(1),
            notes,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), StorageError> {
        if self.notes.len() > 1000 || self.next_id == 0 || self.next_id > 9_007_199_254_740_990 {
            return Err(StorageError::Invalid("library capacity exceeded".into()));
        }

        let mut ids = std::collections::BTreeSet::new();

        for note in &self.notes {
            if note.id == 0
                || note.id >= self.next_id
                || !ids.insert(note.id)
                || note.title.len() > 1024
                || note.body.len() > 512 * 1024
                || note.version == 0
                || note.version > 9_007_199_254_740_990
                || note.modified_ms > 9_007_199_254_740_991
            {
                return Err(StorageError::Invalid(
                    "invalid document identity, version or size".into(),
                ));
            }
        }

        if self.selected.is_some_and(|id| !ids.contains(&id)) {
            return Err(StorageError::Invalid("selected document is missing".into()));
        }

        Ok(())
    }
}

pub(crate) fn now_ms() -> u64 {
    #[cfg(not(target_family = "wasm"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64
    }

    #[cfg(target_family = "wasm")]
    {
        js_sys::Date::now().max(0.) as u64
    }
}
