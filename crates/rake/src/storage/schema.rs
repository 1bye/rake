//! Opt-in JSON schema envelopes and ordered application-owned migrations.

use super::StorageError;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::collections::BTreeMap;

type Step = Box<dyn Fn(Value) -> Result<Value, StorageError>>;

/// A current schema version and JSON migrations from each supported older version.
///
/// Unwrapped JSON is version 0. Versioned JSON reserves the root `$rake_schema`
/// field and uses `{"$rake_schema": 1, "value": ...}`. Steps must be deterministic
/// and free of external side effects: failed reads can run them again.
///
/// ```
/// use rake::storage::{MemoryBackend, Schema, Store};
/// # fn main() -> Result<(), rake::storage::StorageError> {
/// let schema = Schema::new(1).with_migration(0, |value| {
///     let theme: String = serde_json::from_value(value)?;
///     Ok(serde_json::json!({ "theme": theme }))
/// })?;
/// let store = Store::<serde_json::Value>::new(MemoryBackend::new()).with_schema(schema);
/// # Ok(()) }
/// ```
pub struct Schema {
    version: u64,
    steps: BTreeMap<u64, Step>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope<T> {
    #[serde(rename = "$rake_schema")]
    version: u64,
    value: T,
}

impl std::fmt::Debug for Schema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Schema")
            .field("version", &self.version)
            .field("migrations", &self.steps.keys())
            .finish()
    }
}

impl Schema {
    /// Selects the version written by this application; no migrations are registered yet.
    pub fn new(version: u64) -> Self {
        Self {
            version,
            steps: BTreeMap::new(),
        }
    }

    /// Registers one JSON transformation from `from` to `from + 1`.
    ///
    /// Missing steps are reported on read, allowing applications to support only
    /// selected older versions. Closures may capture application configuration.
    ///
    /// # Errors
    /// Returns [`StorageError::InvalidMigration`] for duplicates or `from >= current`.
    pub fn with_migration(
        mut self,
        from: u64,
        step: impl Fn(Value) -> Result<Value, StorageError> + 'static,
    ) -> Result<Self, StorageError> {
        if from >= self.version || self.steps.contains_key(&from) {
            return Err(StorageError::InvalidMigration {
                from,
                current: self.version,
            });
        }

        self.steps.insert(from, Box::new(step));

        Ok(self)
    }

    /// Encodes a value inside the current schema envelope without performing I/O.
    ///
    /// Custom persistence layers may use this codec with their own size limits,
    /// transactions and write-back policy. [`super::Store`] enforces its normal
    /// byte limit including the envelope.
    ///
    /// # Errors
    /// Returns [`StorageError::Codec`] if serialization fails.
    pub fn encode<T: Serialize + ?Sized>(&self, value: &T) -> Result<Vec<u8>, StorageError> {
        Ok(serde_json::to_vec(&Envelope {
            version: self.version,
            value,
        })?)
    }

    /// Migrates JSON and decodes the current type without writing or applying a size limit.
    ///
    /// The boolean reports whether the caller should write back a new envelope,
    /// including unwrapped version-0 data. This is also a fixture test helper:
    /// pass `include_bytes!("old.json")` and assert the returned typed value.
    /// Domain validation and transactional persistence remain the caller's job.
    ///
    /// # Examples
    /// ```
    /// use rake::storage::Schema;
    /// # fn main() -> Result<(), rake::storage::StorageError> {
    /// let schema = Schema::new(1).with_migration(0, |value| {
    ///     let old: String = serde_json::from_value(value)?;
    ///     Ok(serde_json::json!({ "title": old }))
    /// })?;
    /// let fixture = br#"{"$rake_schema":0,"value":"Old note"}"#;
    /// let (note, upgraded) = schema.decode::<serde_json::Value>(fixture)?;
    /// assert!(upgraded);
    /// assert_eq!(note["title"], "Old note");
    /// # Ok(()) }
    /// ```
    ///
    /// # Errors
    /// Rejects newer versions, missing/failing steps, malformed envelopes and values
    /// that do not decode as `T`. Input bytes are never modified.
    pub fn decode<T: DeserializeOwned>(&self, bytes: &[u8]) -> Result<(T, bool), StorageError> {
        let json: Value = serde_json::from_slice(bytes)?;
        let wrapped = json.get("$rake_schema").is_some();
        let (mut version, mut value) = if wrapped {
            let envelope: Envelope<Value> = serde_json::from_value(json)?;

            (envelope.version, envelope.value)
        } else {
            (0, json)
        };

        if version > self.version {
            return Err(StorageError::NewerVersion {
                stored: version,
                current: self.version,
            });
        }

        let changed = !wrapped || version != self.version;

        while version < self.version {
            let step = self
                .steps
                .get(&version)
                .ok_or(StorageError::MissingMigration { from: version })?;

            value = step(value).map_err(|source| StorageError::Migration {
                from: version,
                source: Box::new(source),
            })?;
            version += 1;
        }

        Ok((serde_json::from_value(value)?, changed))
    }
}
