//! Per-user Run registration; no machine-wide keys or StartupApproved writes.
use crate::{Error, LaunchAtLogin, Operation, Status, windows_command};
use std::{io, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::{APPMODEL_ERROR_NO_PACKAGE, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    Storage::Packaging::Appx::GetCurrentPackageFullName,
    System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_BINARY, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    },
};

const APPROVED_KEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: this handle is owned, successfully opened, and closed exactly once.
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

fn check(code: u32) -> Result<(), Error> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32).into())
    }
}

fn open(path: &str, create: bool) -> Result<Option<Key>, Error> {
    let path = wide(path);
    let mut key = ptr::null_mut();
    // SAFETY: UTF-16 path is NUL terminated and all output pointers are valid.
    let code = unsafe {
        if create {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                ptr::null(),
                0,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                ptr::null(),
                &mut key,
                ptr::null_mut(),
            )
        } else {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                &mut key,
            )
        }
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(code)?;

    Ok(Some(Key(key)))
}

fn read(key: &Key, name: &str) -> Result<Option<(u32, Vec<u8>)>, Error> {
    let name = wide(name);
    let mut kind = 0;
    // Run values are limited to 260 characters; allow up to 32K for externally written values.
    let mut data = vec![0; 65536];
    let mut length = data.len() as u32;
    // SAFETY: key is live; all pointers refer to writable sized buffers or NUL-terminated input.
    let code = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            ptr::null(),
            &mut kind,
            data.as_mut_ptr(),
            &mut length,
        )
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(code)?;
    data.truncate(length as usize);

    Ok(Some((kind, data)))
}

fn delete(key: &Key, name: &str) -> Result<(), Error> {
    let name = wide(name);
    // SAFETY: live key and NUL-terminated value name.
    let code = unsafe { RegDeleteValueW(key.0, name.as_ptr()) };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }

    check(code)
}

pub(crate) fn apply(login: &LaunchAtLogin, operation: Operation) -> Result<Status, Error> {
    let mut length = 0;
    // SAFETY: a null buffer with zero length queries package identity without writing data.
    let package = unsafe { GetCurrentPackageFullName(&mut length, ptr::null_mut()) };
    if package != APPMODEL_ERROR_NO_PACKAGE {
        return Ok(Status::Unsupported("MSIX/package identity requires a manifest StartupTask; the Run-key backend is for unpackaged and Velopack apps".into()));
    }

    let executable = std::env::current_exe()?;
    let executable = executable
        .to_str()
        .ok_or_else(|| Error::InvalidCommand("Windows executable path is not Unicode".into()))?;
    let name = windows_command::value_name(login);
    if matches!(operation, Operation::Enable) {
        let command = windows_command::command(login, executable)?;
        let key = open(windows_command::RUN_KEY, true)?
            .ok_or_else(|| Error::Service("Run key creation returned no key".into()))?;
        let command = wide(&command);
        let name = wide(name);
        // SAFETY: both UTF-16 buffers are NUL terminated; length includes the terminator.
        check(unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        })?;
    } else if matches!(operation, Operation::Disable) {
        if let Some(key) = open(windows_command::RUN_KEY, false)? {
            delete(&key, name)?;
        }
        return Ok(Status::Disabled);
    }

    let Some(key) = open(windows_command::RUN_KEY, false)? else {
        return Ok(Status::Disabled);
    };
    if read(&key, name)?.is_none() {
        return Ok(Status::Disabled);
    }
    if let Some(approved) = open(APPROVED_KEY, false)?
        && let Some((REG_BINARY, value)) = read(&approved, name)?
        && value.first().is_some_and(|state| matches!(state, 3 | 7))
    {
        return Ok(Status::RequiresApproval);
    }

    Ok(Status::Enabled)
}

/// Removes an installer-owned Run value only if its command names the installed executable.
///
/// Used by Rake's Velopack before-uninstall callback. Other app IDs, user accounts,
/// and a value redirected to another executable are left untouched. Does not change
/// StartupApproved preferences. Call only for the distribution's validated app ID.
///
/// # Errors
/// Returns invalid-ID, path encoding, or registry access errors.
pub fn remove_installed(app_id: &str, executable: &Path) -> Result<(), Error> {
    let login = LaunchAtLogin::new(app_id)?;
    let executable = executable
        .to_str()
        .ok_or_else(|| Error::InvalidCommand("Windows executable path is not Unicode".into()))?;
    let prefix = windows_command::command(&login, executable)?;
    let Some(key) = open(windows_command::RUN_KEY, false)? else {
        return Ok(());
    };
    let Some((REG_SZ, data)) = read(&key, app_id)? else {
        return Ok(());
    };
    let utf16 = data
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .take_while(|ch| *ch != 0)
        .collect::<Vec<_>>();
    let command =
        String::from_utf16(&utf16).map_err(|error| Error::InvalidCommand(error.to_string()))?;
    if windows_command::belongs_to(&command, &prefix) {
        delete(&key, app_id)?;
    }

    Ok(())
}
