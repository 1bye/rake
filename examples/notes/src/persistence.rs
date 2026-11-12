//! Atomic native files and serialized browser transactions with recovery checkpoints.

use crate::Library;
use rake::storage::Schema;

const MAX_BYTES: usize = 8 * 1024 * 1024;

/// Expected persistence failures; data is never silently replaced with an empty library.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StorageError {
    /// File or browser storage access failed.
    #[error("{0}")]
    Io(String),
    /// Another native instance owns this library.
    #[error("This library is already open in another Fieldnotes instance.")]
    Locked,
    /// A concurrent writer committed data this instance has not loaded.
    #[error("This library changed in another window. Restart Fieldnotes before editing it.")]
    Conflict,
    /// The format belongs to a newer application.
    #[error(
        "Library format {0} is newer than this version of Fieldnotes. Your files were preserved."
    )]
    Unsupported(u64),
    /// Corrupt, oversized or invalid stored data.
    #[error("{0}")]
    Invalid(String),
}

/// Loaded documents and an explicit recovery/migration notice.
#[derive(Clone, Debug)]
pub struct LoadReport {
    /// Validated owned data.
    pub library: Library,
    /// Latest known disk/browser revision.
    pub revision: u64,
    /// Recovery, migration or repaired-file explanation.
    pub notice: Option<String>,
    /// Fresh or migrated data needs a normal save.
    pub needs_save: bool,
}

#[rake::data]
#[derive(Clone, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope {
    pub revision: u64,
    pub writer: String,
    pub library: Library,
}

impl Envelope {
    fn encode(&self) -> Result<String, StorageError> {
        self.library.validate()?;
        let bytes = schema()?.encode(self).map_err(schema_error)?;

        let bytes =
            String::from_utf8(bytes).map_err(|error| StorageError::Invalid(error.to_string()))?;

        if bytes.len() > MAX_BYTES {
            return Err(StorageError::Invalid(
                "Library exceeds 8 MiB. Copy large documents elsewhere and shorten them before adding more."
                    .into(),
            ));
        }

        Ok(bytes)
    }
}

// The schema wraps the complete revisioned snapshot; recovery and compare-and-write
// remain repository transactions rather than Store single-key operations.
fn schema() -> Result<Schema, StorageError> {
    Schema::new(1)
        .with_migration(0, |value| {
            let envelope = legacy_envelope(value)
                .map_err(|error| rake::storage::StorageError::invalid_data(error.to_string()))?;

            Ok(rake::serde_json::to_value(envelope)?)
        })
        .map_err(schema_error)
}

fn schema_error(error: rake::storage::StorageError) -> StorageError {
    if let rake::storage::StorageError::Migration { source, .. } = &error
        && let rake::storage::StorageError::InvalidData(message) = source.as_ref()
    {
        return StorageError::Invalid(message.clone());
    }

    match error {
        rake::storage::StorageError::NewerVersion { stored, .. } => {
            StorageError::Unsupported(stored)
        }
        error => StorageError::Invalid(error.to_string()),
    }
}

fn legacy_envelope(value: rake::serde_json::Value) -> Result<Envelope, StorageError> {
    let format = value
        .get("format")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| StorageError::Invalid("Missing library format.".into()))?;

    if format > 1 {
        return Err(StorageError::Unsupported(format));
    }

    let envelope = if format == 0 {
        #[rake::data]
        struct Legacy {
            format: u64,
            revision: u64,
            notes: Vec<LegacyNote>,
            selected: Option<u64>,
        }

        #[rake::data]
        struct LegacyNote {
            id: u64,
            title: String,
            text: String,
        }
        let legacy: Legacy = rake::serde_json::from_value(value)
            .map_err(|error| StorageError::Invalid(error.to_string()))?;

        let _ = legacy.format;

        let next_id = legacy
            .notes
            .iter()
            .map(|note| note.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| StorageError::Invalid("Document IDs exhausted.".into()))?;

        Envelope {
            revision: legacy.revision,
            writer: "legacy".into(),
            library: Library {
                next_id,
                selected: legacy.selected,
                notes: legacy
                    .notes
                    .into_iter()
                    .map(|note| crate::Note {
                        id: note.id,
                        title: note.title,
                        body: note.text,
                        pinned: false,
                        archived: false,
                        modified_ms: 0,
                        version: 1,
                    })
                    .collect(),
            },
        }
    } else {
        let mut value = value;

        value
            .as_object_mut()
            .ok_or_else(|| StorageError::Invalid("Invalid library.".into()))?
            .remove("format");

        rake::serde_json::from_value(value)
            .map_err(|error| StorageError::Invalid(error.to_string()))?
    };

    Ok(envelope)
}

fn decode(raw: &str) -> Result<(Envelope, bool), StorageError> {
    if raw.len() > MAX_BYTES {
        return Err(StorageError::Invalid(
            "Stored library exceeds 8 MiB.".into(),
        ));
    }

    // Preserve the existing explicit downgrade error for pre-envelope snapshots.
    let value: rake::serde_json::Value = rake::serde_json::from_str(raw)
        .map_err(|error| StorageError::Invalid(format!("Cannot read library: {error}")))?;

    if value.get("$rake_schema").is_none()
        && let Some(format) = value.get("format").and_then(|v| v.as_u64())
        && format > 1
    {
        return Err(StorageError::Unsupported(format));
    }

    let (envelope, migrated) = schema()?
        .decode::<Envelope>(raw.as_bytes())
        .map_err(schema_error)?;

    envelope.library.validate()?;

    if envelope.revision == 0
        || envelope.revision > 9_007_199_254_740_990
        || envelope.writer.is_empty()
    {
        return Err(StorageError::Invalid(
            "Invalid library revision or writer.".into(),
        ));
    }

    Ok((envelope, migrated))
}

fn loaded(
    main: Option<String>,
    recovery: Option<String>,
    previous: Option<String>,
) -> Result<LoadReport, StorageError> {
    let any = main.is_some() || recovery.is_some() || previous.is_some();
    let mut valid = Vec::new();
    let mut damaged = false;

    for (name, raw) in [("saved", main), ("recovery", recovery)] {
        if let Some(raw) = raw {
            match decode(&raw) {
                Ok((envelope, migrated)) => valid.push((name, envelope, migrated)),
                Err(error @ StorageError::Unsupported(_)) => return Err(error),
                Err(_) => damaged = true,
            }
        }
    }

    if valid.is_empty()
        && let Some(raw) = previous
    {
        let (envelope, migrated) = decode(&raw)?;

        valid.push(("previous", envelope, migrated));
        damaged = true;
    }

    let main_revision = valid
        .iter()
        .find(|(name, _, _)| *name == "saved")
        .map(|(_, envelope, _)| envelope.revision);

    valid.sort_by_key(|(_, envelope, _)| envelope.revision);

    if let Some((source, envelope, migrated)) = valid.pop() {
        let recovered = source != "saved" && main_revision != Some(envelope.revision);

        let notice = if migrated {
            Some("Your library was upgraded. The previous version is kept as a backup.".into())
        } else if recovered {
            Some(
                "Recovered your notes after an interrupted save. Your recovered changes are ready."
                    .into(),
            )
        } else if damaged {
            Some("A damaged save was found. Your latest valid notes were preserved.".into())
        } else {
            None
        };

        Ok(LoadReport {
            library: envelope.library,
            revision: envelope.revision,
            notice,
            needs_save: migrated || recovered || damaged,
        })
    } else if !any {
        Ok(LoadReport {
            library: Library::welcome(),
            revision: 0,
            notice: None,
            needs_save: true,
        })
    } else {
        Err(StorageError::Invalid("No readable library or recovery checkpoint was found. Your files were preserved; restore a backup before retrying.".into()))
    }
}

fn current_snapshot(
    main: Option<&str>,
    recovery: Option<&str>,
    previous: Option<&str>,
) -> Result<Option<Envelope>, StorageError> {
    let mut current: Option<Envelope> = None;

    for raw in [main, recovery].into_iter().flatten() {
        match decode(raw) {
            Ok((envelope, _)) => {
                if current
                    .as_ref()
                    .is_none_or(|old| envelope.revision >= old.revision)
                {
                    current = Some(envelope);
                }
            }
            Err(error @ StorageError::Unsupported(_)) => return Err(error),
            Err(_) => {}
        }
    }

    if current.is_none()
        && let Some(previous) = previous
    {
        current = Some(decode(previous)?.0);
    }

    Ok(current)
}

fn check_write(
    current: Option<&Envelope>,
    new: &Envelope,
    expected: u64,
) -> Result<(), StorageError> {
    if let Some(old) = current {
        if old.revision == new.revision && old.library == new.library {
            return Ok(());
        }

        if old.revision > new.revision || (old.revision != expected && old.writer != new.writer) {
            return Err(StorageError::Conflict);
        }
    } else if expected != 0 {
        return Err(StorageError::Conflict);
    }

    Ok(())
}

/// One library connection. Native clones retain the exclusive file lock; Web writes
/// use Web Locks and optimistic revisions. The GUI never performs native I/O directly.
#[derive(Clone)]
pub struct Repository {
    writer: String,
    #[cfg(not(target_family = "wasm"))]
    native: std::sync::Arc<native::Files>,
    #[cfg(target_family = "wasm")]
    key: String,
}

impl Repository {
    /// Uses the platform application-data directory; an explicit native
    /// `RAKE_NOTES_DATA_DIR` selects a separate library for development/testing.
    pub fn application() -> Result<Self, StorageError> {
        #[cfg(not(target_family = "wasm"))]
        {
            let root = if let Some(root) = std::env::var_os("RAKE_NOTES_DATA_DIR") {
                std::path::PathBuf::from(root)
            } else {
                let home = std::env::var_os("HOME")
                    .ok_or_else(|| StorageError::Io("Home directory is unavailable.".into()))?;

                #[cfg(target_os = "macos")]
                let root = std::path::PathBuf::from(home)
                    .join("Library/Application Support/dev.rake.fieldnotes");

                #[cfg(not(target_os = "macos"))]
                let root = std::env::var_os("XDG_DATA_HOME")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::path::PathBuf::from(home).join(".local/share"))
                    .join("fieldnotes");

                root
            };

            Ok(Self::at(root))
        }

        #[cfg(target_family = "wasm")]
        {
            Ok(Self {
                writer: uuid::Uuid::new_v4().to_string(),
                key: "dev.rake.fieldnotes".into(),
            })
        }
    }

    /// Opens an explicit dedicated native directory. I/O and lock acquisition happen on load.
    #[cfg(not(target_family = "wasm"))]
    pub fn at(directory: impl Into<std::path::PathBuf>) -> Self {
        Self {
            writer: uuid::Uuid::new_v4().to_string(),
            native: std::sync::Arc::new(native::Files::new(directory.into())),
        }
    }

    /// Loads the newest validated main/recovery snapshot, or the previous saved copy.
    pub async fn load(&self) -> Result<LoadReport, StorageError> {
        #[cfg(not(target_family = "wasm"))]
        {
            let files = self.native.clone();

            blocking::unblock(move || files.load()).await
        }

        #[cfg(target_family = "wasm")]
        {
            let raw = browser::read(&self.key).await?;

            loaded(raw.0, raw.1, raw.2)
        }
    }

    pub(crate) async fn write(
        &self,
        library: Library,
        revision: u64,
        expected: u64,
        checkpoint_only: bool,
    ) -> Result<(), StorageError> {
        let envelope = Envelope {
            revision,
            writer: self.writer.clone(),
            library,
        };

        let raw = envelope.encode()?;

        #[cfg(not(target_family = "wasm"))]
        {
            let files = self.native.clone();

            blocking::unblock(move || files.write(envelope, raw, expected, checkpoint_only)).await
        }

        #[cfg(target_family = "wasm")]
        {
            browser::write(&self.key, &envelope, &raw, expected, checkpoint_only).await
        }
    }
}

#[cfg(target_family = "wasm")]
mod browser;
#[cfg(not(target_family = "wasm"))]
mod native;

#[cfg(test)]
mod tests {
    //! Covers: preserving application validation messages across schema migration errors.
    //! Real: schema codec and Notes decoder. Doubles: none. Requires: none.

    use super::{StorageError, decode};

    #[test]
    fn legacy_migration_keeps_the_application_error_message() {
        assert_eq!(
            decode("{}").unwrap_err(),
            StorageError::Invalid("Missing library format.".into()),
        );
        let exhausted = r#"{"format":0,"revision":1,"selected":null,"notes":[{"id":18446744073709551615,"title":"Old","text":"Keep"}]}"#;

        assert_eq!(
            decode(exhausted).unwrap_err(),
            StorageError::Invalid("Document IDs exhausted.".into()),
        );
    }
}
