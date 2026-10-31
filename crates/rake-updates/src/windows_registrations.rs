//! Per-user Shell registrations owned by Velopack installer callbacks.
use crate::{Error, UpdateBinding};
use rake_dist_core::DesktopMetadata;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::{io, path::Path};
use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
use winreg::{
    RegKey,
    enums::{HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
};

const OWNER: &str = "Rake.Owner";
fn optional<T>(result: io::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
fn open(classes: &RegKey, name: &str) -> io::Result<Option<RegKey>> {
    optional(classes.open_subkey_with_flags(name, KEY_READ | KEY_WRITE))
}
fn command(executable: &Path) -> Result<String, Error> {
    let path = executable
        .to_str()
        .ok_or_else(|| Error::Policy("Windows executable path is not Unicode".into()))?;
    Ok(format!("\"{path}\" \"%1\""))
}
fn owned(key: &RegKey, identifier: &str, command: &str) -> io::Result<bool> {
    Ok(
        optional(key.get_value::<String, _>(OWNER))?.as_deref() == Some(identifier)
            && optional(key.open_subkey("shell\\open\\command"))?
                .map(|key| optional(key.get_value::<String, _>("")))
                .transpose()?
                .flatten()
                .as_deref()
                == Some(command),
    )
}
fn ensure_owned_or_absent(
    classes: &RegKey,
    name: &str,
    identifier: &str,
    command: &str,
) -> Result<(), Error> {
    if let Some(key) = open(classes, name)?
        && !owned(&key, identifier, command)?
    {
        return Err(Error::Policy(format!(
            "Windows registration {name} belongs to another application"
        )));
    }
    Ok(())
}
fn handler(
    classes: &RegKey,
    name: &str,
    label: &str,
    identifier: &str,
    executable: &Path,
    command: &str,
) -> Result<RegKey, Error> {
    let (key, _) = classes.create_subkey(name)?;
    key.set_value(OWNER, &identifier)?;
    key.set_value("", &label)?;
    key.create_subkey("DefaultIcon")?
        .0
        .set_value("", &format!("\"{}\",0", executable.display()))?;
    key.create_subkey("shell\\open\\command")?
        .0
        .set_value("", &command)?;
    Ok(key)
}
fn install_handlers(
    classes: &RegKey,
    merged: &RegKey,
    identifier: &str,
    desktop: &DesktopMetadata,
    executable: &Path,
) -> Result<(), Error> {
    let command = command(executable)?;
    for scheme in &desktop.url_schemes {
        handler(
            classes,
            scheme,
            &format!("URL:{identifier}"),
            identifier,
            executable,
            &command,
        )?
        .set_value("URL Protocol", &"")?;
    }
    for association in &desktop.file_associations {
        for extension in &association.extensions {
            let progid = format!("{identifier}.{extension}");
            let label = association
                .description
                .as_deref()
                .or(association.name.as_deref())
                .unwrap_or(identifier);
            handler(classes, &progid, label, identifier, executable, &command)?;
            let extension = format!(".{extension}");
            let existing = optional(merged.open_subkey(&extension))?
                .map(|key| optional(key.get_value::<String, _>("")))
                .transpose()?
                .flatten();
            let (key, _) = classes.create_subkey(&extension)?;
            if existing.as_deref().is_none_or(str::is_empty) {
                key.set_value("", &progid)?;
            }
            key.create_subkey("OpenWithProgids")?
                .0
                .set_value(&progid, &"")?;
        }
    }
    Ok(())
}
fn remove_handlers(
    classes: &RegKey,
    identifier: &str,
    desktop: &DesktopMetadata,
    executable: &Path,
) -> Result<(), Error> {
    let command = command(executable)?;
    for scheme in &desktop.url_schemes {
        if let Some(key) = open(classes, scheme)?
            && owned(&key, identifier, &command)?
        {
            classes.delete_subkey_all(scheme)?;
        }
    }
    for association in &desktop.file_associations {
        for extension in &association.extensions {
            let progid = format!("{identifier}.{extension}");
            let Some(handler) = open(classes, &progid)? else {
                continue;
            };
            if !owned(&handler, identifier, &command)? {
                continue;
            }
            if let Some(key) = open(classes, &format!(".{extension}"))? {
                if optional(key.get_value::<String, _>(""))?.as_deref() == Some(&progid) {
                    optional(key.delete_value(""))?;
                }
                if let Some(choices) =
                    optional(key.open_subkey_with_flags("OpenWithProgids", KEY_READ | KEY_WRITE))?
                {
                    optional(choices.delete_value(&progid))?;
                }
            }
            classes.delete_subkey_all(&progid)?;
        }
    }
    Ok(())
}
// Setup replacement does not always execute the obsolete version's callback.
// Keep the prior declarations outside Shell handler keys to reconcile them here.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RegistrationRecord {
    schema_version: u32,
    desktop: DesktopMetadata,
    executable: PathBuf,
}
fn record_name(identifier: &str) -> String {
    // Underscores cannot occur in configured URI schemes or application identifiers.
    format!("Rake_Registrations.{identifier}")
}
fn previous(classes: &RegKey, identifier: &str) -> Result<Option<RegistrationRecord>, Error> {
    let Some(key) = open(classes, &record_name(identifier))? else {
        return Ok(None);
    };
    if optional(key.get_value::<String, _>(OWNER))?.as_deref() != Some(identifier) {
        return Err(Error::Policy(
            "Windows registration record belongs to another application".into(),
        ));
    }
    let data: String = key.get_value("Metadata")?;
    if data.len() > 1024 * 1024 {
        return Err(Error::Policy(
            "Windows registration record exceeds size limit".into(),
        ));
    }
    let record: RegistrationRecord = serde_json::from_str(&data)?;
    record
        .desktop
        .validate()
        .map_err(|error| Error::Policy(error.to_string()))?;
    if record.schema_version != 1 || !record.executable.is_absolute() {
        return Err(Error::Policy("invalid Windows registration record".into()));
    }
    Ok(Some(record))
}
fn install(
    classes: &RegKey,
    merged: &RegKey,
    identifier: &str,
    desktop: &DesktopMetadata,
    executable: &Path,
) -> Result<(), Error> {
    let command = command(executable)?;
    // Preflight every exclusive key before mutating any registration.
    for scheme in &desktop.url_schemes {
        ensure_owned_or_absent(classes, scheme, identifier, &command)?;
    }
    for association in &desktop.file_associations {
        for extension in &association.extensions {
            ensure_owned_or_absent(
                classes,
                &format!("{identifier}.{extension}"),
                identifier,
                &command,
            )?;
        }
    }
    if let Some(record) = previous(classes, identifier)? {
        remove_handlers(classes, identifier, &record.desktop, &record.executable)?;
    }
    install_handlers(classes, merged, identifier, desktop, executable)?;
    let record = RegistrationRecord {
        schema_version: 1,
        desktop: desktop.clone(),
        executable: executable.into(),
    };
    let (key, _) = classes.create_subkey(record_name(identifier))?;
    key.set_value(OWNER, &identifier)?;
    key.set_value("Metadata", &serde_json::to_string(&record)?)?;
    Ok(())
}
fn uninstall(
    classes: &RegKey,
    identifier: &str,
    desktop: &DesktopMetadata,
    executable: &Path,
) -> Result<(), Error> {
    if let Some(record) = previous(classes, identifier)? {
        remove_handlers(classes, identifier, &record.desktop, &record.executable)?;
        classes.delete_subkey_all(record_name(identifier))?;
    }
    remove_handlers(classes, identifier, desktop, executable)
}
fn update_registrations(
    binding: &UpdateBinding,
    desktop: &DesktopMetadata,
    remove: bool,
) -> Result<(), Error> {
    if !binding.identifier.contains('.')
        || !binding.identifier.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return Err(Error::Policy(
            "invalid Windows registration identifier".into(),
        ));
    }
    let classes = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey("Software\\Classes")?
        .0;
    if desktop.url_schemes.is_empty()
        && desktop.file_associations.is_empty()
        && previous(&classes, &binding.identifier)?.is_none()
    {
        return Ok(());
    }
    let location = ::velopack::locator::auto_locate_app_manifest(
        ::velopack::locator::LocationContext::FromCurrentExe,
    )
    .map_err(|error| Error::Provider(error.to_string()))?;
    if location.get_is_portable() {
        return Ok(());
    }
    if location.get_manifest_id() != binding.identifier {
        return Err(Error::Policy(
            "Velopack registration identity mismatch".into(),
        ));
    }
    let executable = location.get_main_exe_path();
    if remove {
        uninstall(&classes, &binding.identifier, desktop, &executable)?;
    } else {
        install(
            &classes,
            &RegKey::predef(HKEY_CLASSES_ROOT),
            &binding.identifier,
            desktop,
            &executable,
        )?;
    }
    // SAFETY: association-changed uses no payload pointers; the Shell refreshes its cache.
    unsafe {
        SHChangeNotify(
            SHCNE_ASSOCCHANGED as i32,
            SHCNF_IDLIST,
            std::ptr::null(),
            std::ptr::null(),
        );
    }
    Ok(())
}
pub(crate) fn startup(binding: &UpdateBinding, desktop: &DesktopMetadata) {
    let apply = |remove| {
        if let Err(error) = update_registrations(binding, desktop, remove) {
            eprintln!("Rake installer registration failed: {error}");
            std::process::exit(1);
        }
    };
    ::velopack::VelopackApp::build()
        .set_auto_apply_on_startup(false)
        .on_after_install_fast_callback(|_| apply(false))
        .on_before_update_fast_callback(|_| apply(true))
        .on_after_update_fast_callback(|_| apply(false))
        .on_before_uninstall_fast_callback(|_| apply(true))
        .on_first_run(|_| apply(false))
        .on_restarted(|_| apply(false))
        .run();
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn setup_replacement_removes_obsolete_declarations_even_when_the_new_set_is_empty() {
        let root = format!("Software\\RakeTests\\replacement-{}", std::process::id());
        let user = RegKey::predef(HKEY_CURRENT_USER);
        assert!(optional(user.open_subkey(&root)).unwrap().is_none());
        let (classes, _) = user.create_subkey(&root).unwrap();
        let result = std::panic::catch_unwind(|| {
            let old: DesktopMetadata = serde_json::from_str(
                r#"{"url_schemes":["rake-old"],"file_associations":[{"extensions":["rakeold"]}]}"#,
            )
            .unwrap();
            let new: DesktopMetadata =
                serde_json::from_str(r#"{"url_schemes":["rake-new"]}"#).unwrap();
            let executable = Path::new("C:\\Rake\\current\\app.exe");
            install(&classes, &classes, "dev.rake.test", &old, executable).unwrap();
            install(&classes, &classes, "dev.rake.test", &new, executable).unwrap();
            assert!(optional(classes.open_subkey("rake-old")).unwrap().is_none());
            assert!(
                optional(classes.open_subkey("dev.rake.test.rakeold"))
                    .unwrap()
                    .is_none()
            );
            assert!(optional(classes.open_subkey("rake-new")).unwrap().is_some());
            install(
                &classes,
                &classes,
                "dev.rake.test",
                &DesktopMetadata::default(),
                executable,
            )
            .unwrap();
            assert!(optional(classes.open_subkey("rake-new")).unwrap().is_none());
            uninstall(&classes, "dev.rake.test", &new, executable).unwrap();
            assert!(previous(&classes, "dev.rake.test").unwrap().is_none());
        });
        drop(classes);
        user.delete_subkey_all(root).unwrap();
        result.unwrap();
    }
    #[test]
    fn registrations_quote_paths_and_preserve_foreign_handlers_and_user_defaults() {
        let root = format!("Software\\RakeTests\\registrations-{}", std::process::id());
        let user = RegKey::predef(HKEY_CURRENT_USER);
        assert!(optional(user.open_subkey(&root)).unwrap().is_none());
        let (classes, _) = user.create_subkey(&root).unwrap();
        let result = std::panic::catch_unwind(|| {
            let desktop: DesktopMetadata = serde_json::from_str(r#"{"url_schemes":["rake-reg-test"],"file_associations":[{"extensions":["rktest","rkshared"],"name":"Rake test"}]}"#).unwrap();
            let executable = Path::new("C:\\Program Files\\Rake App\\current\\app.exe");
            classes
                .create_subkey(".rkshared")
                .unwrap()
                .0
                .set_value("", &"other.application")
                .unwrap();
            install(&classes, &classes, "dev.rake.test", &desktop, executable).unwrap();
            assert_eq!(
                classes
                    .open_subkey(".rkshared")
                    .unwrap()
                    .get_value::<String, _>("")
                    .unwrap(),
                "other.application"
            );
            let key = classes
                .open_subkey("rake-reg-test\\shell\\open\\command")
                .unwrap();
            assert_eq!(
                key.get_value::<String, _>("").unwrap(),
                "\"C:\\Program Files\\Rake App\\current\\app.exe\" \"%1\""
            );
            install(&classes, &classes, "dev.rake.test", &desktop, executable).unwrap();
            classes
                .open_subkey_with_flags(".rktest", KEY_WRITE)
                .unwrap()
                .set_value("", &"other.application")
                .unwrap();
            uninstall(&classes, "dev.rake.test", &desktop, executable).unwrap();
            assert!(
                optional(classes.open_subkey("rake-reg-test"))
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                classes
                    .open_subkey(".rktest")
                    .unwrap()
                    .get_value::<String, _>("")
                    .unwrap(),
                "other.application"
            );
            classes
                .create_subkey("rake-reg-test\\shell\\open\\command")
                .unwrap()
                .0
                .set_value("", &"foreign command")
                .unwrap();
            assert!(install(&classes, &classes, "dev.rake.test", &desktop, executable).is_err());
            uninstall(&classes, "dev.rake.test", &desktop, executable).unwrap();
            assert_eq!(
                classes
                    .open_subkey("rake-reg-test\\shell\\open\\command")
                    .unwrap()
                    .get_value::<String, _>("")
                    .unwrap(),
                "foreign command"
            );
        });
        drop(classes);
        user.delete_subkey_all(root).unwrap();
        result.unwrap();
    }
}
