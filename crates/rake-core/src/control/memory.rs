//! Capability-gated dispatch shared by routed and standalone native hosts.

use super::{ControlError, Reply};

pub(super) fn available() -> bool {
    #[cfg(all(feature = "devtools-memory", unix))]
    {
        crate::memory::installed()
    }

    #[cfg(not(all(feature = "devtools-memory", unix)))]
    {
        false
    }
}

pub(super) fn record(
    owner: &str,
    enabled: bool,
    interval: u64,
    lease_ms: u64,
) -> Result<Reply, ControlError> {
    #[cfg(all(feature = "devtools-memory", unix))]
    {
        crate::memory::record(owner, enabled, interval, lease_ms)
            .map_err(|reason| ControlError::Memory { reason })?;

        Ok(Reply::Acknowledged)
    }

    #[cfg(not(all(feature = "devtools-memory", unix)))]
    {
        let _ = (owner, enabled, interval, lease_ms);

        Err(ControlError::Unsupported {
            operation: "native memory recording".into(),
        })
    }
}

pub(super) fn snapshot(owner: &str, include_freed: bool) -> Result<Reply, ControlError> {
    #[cfg(all(feature = "devtools-memory", unix))]
    {
        crate::memory::snapshot(owner, include_freed)
            .map(Reply::Memory)
            .map_err(|reason| ControlError::Memory { reason })
    }

    #[cfg(not(all(feature = "devtools-memory", unix)))]
    {
        let _ = (owner, include_freed);

        Err(ControlError::Unsupported {
            operation: "native memory recording".into(),
        })
    }
}
