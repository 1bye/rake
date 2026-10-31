//! Hidden entry-point adapter for infallible and fallible application factories.
//!
//! This module is intentionally unreferenced until the generated entry points are
//! switched to [`run_app_factory`]. Keeping the adapter isolated lets the runner
//! change land independently from the host bootstrap implementation.

use std::fmt::Display;

use super::{App, Route};

mod sealed {
    pub trait Sealed {}
}

/// Output accepted by the hidden `#[rake::app]` entry-point adapter.
#[doc(hidden)]
pub trait AppFactoryOutput: sealed::Sealed {
    /// Applies generated distribution metadata and runs the application host.
    fn run_with_policy(
        self,
        policy: Option<&'static str>,
        resources: &'static str,
    ) -> Result<(), String>;
}

impl<R: Route> sealed::Sealed for App<R> {}

impl<R: Route> AppFactoryOutput for App<R> {
    fn run_with_policy(
        self,
        policy: Option<&'static str>,
        resources: &'static str,
    ) -> Result<(), String> {
        self.distribution_policy(policy, resources)
            .run()
            .map_err(|error| error.to_string())
    }
}

impl<R: Route, E: Display> sealed::Sealed for Result<App<R>, E> {}

impl<R: Route, E: Display> AppFactoryOutput for Result<App<R>, E> {
    fn run_with_policy(
        self,
        policy: Option<&'static str>,
        resources: &'static str,
    ) -> Result<(), String> {
        let app = self.map_err(|error| error.to_string())?;

        app.run_with_policy(policy, resources)
    }
}

/// Runs either an existing `App` factory or a factory returning `Result<App, E>`.
#[doc(hidden)]
pub fn run_app_factory<F, T>(
    factory: F,
    policy: Option<&'static str>,
    resources: &'static str,
) -> Result<(), String>
where
    F: FnOnce() -> T,
    T: AppFactoryOutput,
{
    let result = factory().run_with_policy(policy, resources);
    #[cfg(target_family = "wasm")]
    match &result {
        Err(error) => super::startup::show_browser_error(error),
        Ok(()) => super::startup::clear_browser_error(),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FactoryError;

    impl Display for FactoryError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("factory failed")
        }
    }

    #[test]
    fn infallible_factory_preserves_app_startup_errors() {
        let result = run_app_factory(|| App::new(""), None, "factory-test");

        assert_eq!(
            result,
            Err("application ID must be nonempty and contain no whitespace".into())
        );
    }

    #[test]
    fn fallible_factory_reports_factory_errors() {
        let result = run_app_factory(|| Err::<App, _>(FactoryError), None, "factory-test");

        assert_eq!(result, Err("factory failed".into()));
    }

    #[test]
    fn fallible_factory_preserves_app_startup_errors() {
        let result = run_app_factory(
            || Ok::<App, FactoryError>(App::new("")),
            None,
            "factory-test",
        );

        assert_eq!(
            result,
            Err("application ID must be nonempty and contain no whitespace".into())
        );
    }
}
