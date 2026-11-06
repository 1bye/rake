//! Shell registration sequencing, separate from HWND ownership for deterministic tests.
use crate::HostPolicy;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Command {
    Add,
    SetVersion,
    Delete,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Failure {
    Add,
    Version,
}
impl Failure {
    pub(super) fn operation(self) -> &'static str {
        match self {
            Self::Add => "add tray icon",
            Self::Version => "negotiate tray version",
        }
    }
}

pub(super) fn register(
    policy: HostPolicy,
    mut call: impl FnMut(Command) -> bool,
) -> Result<bool, Failure> {
    if !call(Command::Add) {
        return if policy == HostPolicy::Wait {
            Ok(false)
        } else {
            Err(Failure::Add)
        };
    }
    if !call(Command::SetVersion) {
        // Never leave a live legacy-format registration feeding the v4 decoder.
        call(Command::Delete);
        return Err(Failure::Version);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    //! Covers: initial shell absence, later recovery and version negotiation rollback.
    //! Real: registration policy/sequencing. Doubles: scripted Shell return values.
    //! Requires: no Windows desktop.
    use super::*;
    #[test]
    fn wait_keeps_an_offline_item_that_can_register_after_shell_recovery() {
        let mut commands = Vec::new();
        assert!(
            !register(HostPolicy::Wait, |command| {
                commands.push(command);
                false
            })
            .unwrap()
        );
        assert_eq!(commands, [Command::Add]);
        commands.clear();
        assert!(
            register(HostPolicy::Wait, |command| {
                commands.push(command);
                true
            })
            .unwrap()
        );
        assert_eq!(commands, [Command::Add, Command::SetVersion]);
    }
    #[test]
    fn require_rejects_initial_absence_and_negotiation_failure_removes_partial_registration() {
        let failure = register(HostPolicy::Require, |_| false).unwrap_err();
        assert_eq!(failure.operation(), "add tray icon");
        let mut commands = Vec::new();
        let failure = register(HostPolicy::Wait, |command| {
            commands.push(command);
            command != Command::SetVersion
        })
        .unwrap_err();
        assert_eq!(failure.operation(), "negotiate tray version");
        assert_eq!(
            commands,
            [Command::Add, Command::SetVersion, Command::Delete]
        );
    }
}
