//! Shell registration sequencing, separate from HWND ownership for deterministic tests.
use crate::HostPolicy;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Identity {
    DerivedGuid,
    ExplicitGuid,
    Window,
}
impl Identity {
    pub(super) fn uses_guid(self) -> bool {
        self != Self::Window
    }
}
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
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Registered {
    pub(super) online: bool,
    pub(super) fell_back: bool,
}

pub(super) fn register(
    policy: HostPolicy,
    identity: &mut Identity,
    mut call: impl FnMut(Command, Identity) -> bool,
) -> Result<Registered, Failure> {
    let mut fell_back = false;
    if !call(Command::Add, *identity) {
        if *identity == Identity::DerivedGuid && call(Command::Add, Identity::Window) {
            *identity = Identity::Window;
            fell_back = true;
        } else {
            return if policy == HostPolicy::Wait {
                Ok(Registered {
                    online: false,
                    fell_back,
                })
            } else {
                Err(Failure::Add)
            };
        }
    }
    if !call(Command::SetVersion, *identity) {
        // Never leave a live legacy-format registration feeding the v4 decoder.
        call(Command::Delete, *identity);
        return Err(Failure::Version);
    }
    Ok(Registered {
        online: true,
        fell_back,
    })
}

#[cfg(test)]
mod tests {
    //! Covers: shell absence/recovery, derived GUID fallback and explicit identity preservation.
    //! Real: registration policy/sequencing. Doubles: scripted Shell return values.
    //! Requires: no Windows desktop.
    use super::*;
    #[test]
    fn wait_keeps_an_offline_item_that_can_register_after_shell_recovery() {
        let mut identity = Identity::ExplicitGuid;
        let mut commands = Vec::new();
        assert!(
            !register(HostPolicy::Wait, &mut identity, |command, _| {
                commands.push(command);
                false
            })
            .unwrap()
            .online
        );
        assert_eq!(commands, [Command::Add]);
        commands.clear();
        assert!(
            register(HostPolicy::Wait, &mut identity, |command, _| {
                commands.push(command);
                true
            })
            .unwrap()
            .online
        );
        assert_eq!(commands, [Command::Add, Command::SetVersion]);
    }
    #[test]
    fn require_rejects_initial_absence_and_negotiation_failure_removes_partial_registration() {
        let mut identity = Identity::ExplicitGuid;
        let failure = register(HostPolicy::Require, &mut identity, |_, _| false).unwrap_err();
        assert_eq!(failure.operation(), "add tray icon");
        let mut commands = Vec::new();
        let failure = register(HostPolicy::Wait, &mut identity, |command, _| {
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
    #[test]
    fn derived_guid_retries_once_and_remembers_window_identity_for_recovery() {
        let mut identity = Identity::DerivedGuid;
        let mut commands = Vec::new();
        let result = register(HostPolicy::Require, &mut identity, |command, mode| {
            commands.push((command, mode));
            mode == Identity::Window
        })
        .unwrap();
        assert_eq!(
            result,
            Registered {
                online: true,
                fell_back: true
            }
        );
        assert_eq!(
            commands,
            [
                (Command::Add, Identity::DerivedGuid),
                (Command::Add, Identity::Window),
                (Command::SetVersion, Identity::Window)
            ]
        );
        assert!(!identity.uses_guid());
        commands.clear();
        let result = register(HostPolicy::Wait, &mut identity, |command, mode| {
            commands.push((command, mode));
            true
        })
        .unwrap();
        assert!(result.online && !result.fell_back);
        assert!(commands.iter().all(|(_, mode)| *mode == Identity::Window));
    }
    #[test]
    fn explicit_guid_never_falls_back_and_missing_shell_does_not_discard_derived_identity() {
        for (original, attempts) in [(Identity::ExplicitGuid, 1), (Identity::DerivedGuid, 2)] {
            let mut identity = original;
            let mut count = 0;
            let result = register(HostPolicy::Wait, &mut identity, |_, _| {
                count += 1;
                false
            })
            .unwrap();
            assert!(!result.online && !result.fell_back);
            assert_eq!(identity, original);
            assert!(identity.uses_guid());
            assert_eq!(count, attempts);
        }
    }
    #[test]
    fn failed_version_negotiation_cleans_up_using_the_fallback_identity() {
        let mut identity = Identity::DerivedGuid;
        let mut commands = Vec::new();
        assert_eq!(
            register(HostPolicy::Wait, &mut identity, |command, mode| {
                commands.push((command, mode));
                mode == Identity::Window && command != Command::SetVersion
            }),
            Err(Failure::Version)
        );
        assert_eq!(identity, Identity::Window);
        assert_eq!(commands.last(), Some(&(Command::Delete, Identity::Window)));
    }
}
