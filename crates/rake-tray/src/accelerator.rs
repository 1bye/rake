//! Menu-scoped key equivalents, independent of global shortcut registration.
use crate::Error;

/// One parsed menu key equivalent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Accelerator {
    /// One printable Unicode key (lowercase for AppKit letter equivalents).
    pub key: String,
    /// Control modifier.
    pub control: bool,
    /// Alt/Option modifier.
    pub alt: bool,
    /// Shift modifier.
    pub shift: bool,
    /// Command/Super modifier.
    pub super_key: bool,
}

impl Accelerator {
    /// Parses `Ctrl+Alt+Shift+Super+k`; modifiers are optional and unique.
    /// `Cmd` aliases `Super`, and `Option` aliases `Alt`.
    /// # Errors
    /// Rejects unknown/duplicate modifiers, controls and keys longer than one character.
    pub fn parse(value: &str) -> Result<Self, Error> {
        let mut parts = value.split('+').collect::<Vec<_>>();
        let key = parts.pop().unwrap_or_default();

        if key.chars().count() != 1 || key.chars().any(char::is_control) {
            return Err(Error::InvalidInput(
                "accelerator requires one printable key".into(),
            ));
        }

        let mut result = Self {
            key: key.to_lowercase(),
            control: false,
            alt: false,
            shift: false,
            super_key: false,
        };

        for part in parts {
            let flag = match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut result.control,
                "alt" | "option" => &mut result.alt,
                "shift" => &mut result.shift,
                "super" | "cmd" => &mut result.super_key,
                _ => {
                    return Err(Error::InvalidInput(format!(
                        "unknown accelerator modifier: {part}"
                    )));
                }
            };

            if *flag {
                return Err(Error::InvalidInput("duplicate accelerator modifier".into()));
            }

            *flag = true;
        }

        Ok(result)
    }
}
