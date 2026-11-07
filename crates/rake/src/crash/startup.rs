//! App-owned crash startup policy, shared with headless subprocess tests.

use std::io;

use super::{CrashConfig, CrashSink};
use crate::diagnostics::Collector;

#[derive(Default)]
pub(crate) enum CrashReports {
    #[default]
    Automatic,
    Disabled,
    Configured(CrashConfig, Box<dyn CrashSink>),
}

impl CrashReports {
    pub(crate) fn configure(&mut self, config: CrashConfig, sink: impl CrashSink) {
        *self = Self::Configured(config, Box::new(sink));
    }

    pub(crate) fn disable(&mut self) {
        *self = Self::Disabled;
    }

    pub(crate) fn initialize(
        self,
        application_id: &str,
        version: Option<&str>,
        collector: Collector,
    ) -> io::Result<()> {
        match self {
            Self::Automatic => {
                let result = CrashConfig::new(version.unwrap_or("unknown"))
                    .open(application_id)
                    .and_then(|reporter| {
                        reporter.install(collector)?;
                        #[cfg(target_family = "wasm")]
                        reporter.prune();
                        Ok(())
                    });

                if let Err(error) = result {
                    tracing::warn!(%error, "default crash recording unavailable; startup continues");
                }
            }
            Self::Disabled => {}
            Self::Configured(config, sink) => {
                let reporter = config.open(application_id)?;
                reporter.install(collector)?;

                #[cfg(not(target_family = "wasm"))]
                if reporter.deliver(sink.as_ref()).is_err() {
                    tracing::warn!("crash delivery failed; waiting record retained");
                }
                #[cfg(target_family = "wasm")]
                wasm_bindgen_futures::spawn_local(async move {
                    if reporter.deliver(sink).await.is_err() {
                        tracing::warn!("crash delivery failed; waiting record retained");
                    }
                });
            }
        }

        Ok(())
    }
}

pub(crate) fn version_from_policy(policy: Option<&str>) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Version {
        version: String,
    }

    serde_json::from_str::<Version>(policy?)
        .ok()
        .map(|policy| policy.version)
        .filter(|version| !version.trim().is_empty())
}
