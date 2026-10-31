//! Generation-safe state for view-owned asynchronous loading.
//!
//! [`LoadState`] does not spawn work or choose an executor. A caller starts a
//! load with [`LoadState::begin`], captures the returned token, and completes
//! only that generation from its task. A late result from an older retry is
//! rejected without touching the current state.
//!
//! ```no_run
//! use rake::loading::{LoadState, LoadStatus};
//!
//! let mut load = LoadState::<String>::default();
//! let token = load.begin();
//! // An application-owned task performs the request and keeps this token.
//! let _accepted = load.complete(token, Ok("ready".into()));
//! assert!(matches!(load.state(), LoadStatus::Ready(value) if value == "ready"));
//! ```

use std::fmt;

#[cfg(feature = "gpui")]
mod binding;
#[cfg(feature = "gpui")]
pub use binding::{LoadContext, LoadError, Loader};

/// One private generation identity returned by [`LoadState::begin`].
///
/// Both fields are private so callers can retain and compare tokens only
/// through the loading state that issued them. The owner UUID also prevents a
/// token from one state instance from completing the first generation of a
/// different instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LoadToken {
    owner: uuid::Uuid,
    generation: u64,
}

/// The observable state of one generation-safe load.
#[derive(Debug)]
pub enum LoadStatus<T, E = String> {
    /// No request is currently active.
    Idle,
    /// A request is active for the returned generation token.
    Loading(LoadToken),
    /// The current request completed successfully.
    Ready(T),
    /// The current request failed.
    Failed(E),
}

/// Owns the current loading status and monotonically increasing generation.
///
/// Results are accepted only while the state is `Loading` for the exact token
/// returned by the matching [`begin`](Self::begin). This makes retry and reset
/// paths safe against late or duplicated task results without requiring `T` or
/// `E` to implement `Clone`.
pub struct LoadState<T, E = String> {
    owner: uuid::Uuid,
    generation: u64,
    status: LoadStatus<T, E>,
}

impl<T, E> Default for LoadState<T, E> {
    fn default() -> Self {
        Self {
            owner: uuid::Uuid::new_v4(),
            generation: 0,
            status: LoadStatus::Idle,
        }
    }
}

impl<T, E> LoadState<T, E> {
    /// Creates an idle loading state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Starts a new generation and returns its private completion token.
    ///
    /// # Panics
    ///
    /// Panics only if the `u64` generation counter is exhausted. A process
    /// cannot realistically perform that many retries; refusing to wrap is
    /// safer than allowing an ancient token to become current again.
    pub fn begin(&mut self) -> LoadToken {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("load generation exhausted");
        let token = LoadToken {
            owner: self.owner,
            generation: self.generation,
        };
        self.status = LoadStatus::Loading(token);
        token
    }

    /// Completes the matching loading generation.
    ///
    /// Returns `true` only when `token` is the current `Loading` token. Stale,
    /// duplicated, and post-reset results return `false` and leave the state
    /// unchanged.
    pub fn complete(&mut self, token: LoadToken, result: Result<T, E>) -> bool {
        if !matches!(&self.status, LoadStatus::Loading(current) if *current == token) {
            return false;
        }

        self.status = match result {
            Ok(value) => LoadStatus::Ready(value),
            Err(error) => LoadStatus::Failed(error),
        };
        true
    }

    /// Returns to `Idle` while retaining the generation counter.
    ///
    /// The old token cannot complete after reset because the state is no
    /// longer loading; the next [`begin`](Self::begin) also advances the
    /// counter before issuing a new token.
    pub fn reset(&mut self) {
        self.status = LoadStatus::Idle;
    }

    /// Returns the current status without moving its value.
    pub fn state(&self) -> &LoadStatus<T, E> {
        &self.status
    }
}

impl<T: fmt::Debug, E: fmt::Debug> fmt::Debug for LoadState<T, E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LoadState")
            .field("generation", &self.generation)
            .field("status", &self.status)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::{LoadState, LoadStatus};

    #[test]
    fn accepts_only_the_current_generation() {
        let mut state = LoadState::<u32>::default();
        let first = state.begin();
        let second = state.begin();

        assert!(!state.complete(first, Ok(1)));
        assert!(matches!(state.state(), LoadStatus::Loading(token) if *token == second));
        assert!(state.complete(second, Ok(2)));
        assert!(matches!(state.state(), LoadStatus::Ready(2)));
    }

    #[test]
    fn rejects_duplicate_completion() {
        let mut state = LoadState::<u32>::default();
        let token = state.begin();

        assert!(state.complete(token, Err("failed".to_owned())));
        assert!(!state.complete(token, Ok(3)));
        assert!(matches!(state.state(), LoadStatus::Failed(error) if error == "failed"));
    }

    #[test]
    fn failed_load_can_retry_with_a_fresh_token() {
        let mut state = LoadState::<u32>::default();
        let first = state.begin();
        assert!(state.complete(first, Err("temporary".to_owned())));

        let retry = state.begin();
        assert_ne!(first, retry);
        assert!(state.complete(retry, Ok(7)));
        assert!(matches!(state.state(), LoadStatus::Ready(7)));
    }

    #[test]
    fn reset_rejects_old_results_and_returns_to_idle() {
        let mut state = LoadState::<u32>::default();
        let old = state.begin();
        state.reset();

        assert!(matches!(state.state(), LoadStatus::Idle));
        assert!(!state.complete(old, Ok(1)));

        let fresh = state.begin();
        assert_ne!(old, fresh);
        assert!(state.complete(fresh, Ok(2)));
    }

    #[test]
    fn rejects_tokens_from_another_state_instance() {
        let mut first = LoadState::<u32>::default();
        let foreign = first.begin();
        let mut second = LoadState::<u32>::default();
        let current = second.begin();

        assert_ne!(foreign, current);
        assert!(!second.complete(foreign, Ok(1)));
        assert!(matches!(second.state(), LoadStatus::Loading(token) if *token == current));
    }

    #[test]
    fn supports_move_only_values_and_errors() {
        struct Value(String);
        struct Failure(String);

        let mut state = LoadState::<Value, Failure>::default();
        let token = state.begin();
        assert!(state.complete(token, Ok(Value("loaded".into()))));

        let LoadStatus::Ready(value) = state.state() else {
            panic!("expected ready");
        };
        assert_eq!(value.0, "loaded");

        let retry = state.begin();
        assert!(state.complete(retry, Err(Failure("offline".into()))));
        let LoadStatus::Failed(error) = state.state() else {
            panic!("expected failed");
        };
        assert_eq!(error.0, "offline");
    }
}
