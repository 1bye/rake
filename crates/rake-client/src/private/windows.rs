//! Windows ownership checks use the opened handle, never a path-based ACL query.

use std::{
    ffi::c_void,
    fs::File,
    io, mem,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    Storage::FileSystem::*,
    System::Threading::*,
};

fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "expected a current-user-owned object with an owner-only DACL and no reparse points",
    )
}

fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let mut value: Vec<u16> = path.as_os_str().encode_wide().collect();

    if value.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }

    value.push(0);

    Ok(value)
}

struct LocalAllocation(*mut c_void);

impl Drop for LocalAllocation {
    fn drop(&mut self) {
        // SAFETY: pointer is null or allocated by an API documenting LocalFree.
        unsafe {
            LocalFree(self.0);
        }
    }
}

fn user() -> io::Result<Vec<usize>> {
    let mut token = ptr::null_mut();
    // SAFETY: output points to a live handle slot; current process is always valid.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: OpenProcessToken transferred ownership of this handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut size = 0;
    // SAFETY: the null buffer requests the required length.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut size,
        );
    }
    let mut buffer = vec![0usize; (size as usize).div_ceil(mem::size_of::<usize>())];
    // SAFETY: buffer has at least size bytes and TOKEN_USER alignment.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }

    Ok(buffer)
}

/// Owns a protected security descriptor granting full access only to the current user.
/// Used for both filesystem creation and local named-pipe creation.
pub(crate) struct Security(LocalAllocation);

impl Security {
    /// Builds a descriptor using the process token's user SID.
    ///
    /// # Errors
    /// Returns token or security-descriptor API failures.
    pub(crate) fn current_user() -> io::Result<Self> {
        let user = user()?;
        // SAFETY: user contains a live TOKEN_USER returned by GetTokenInformation.
        let sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
        let mut text = ptr::null_mut();
        // SAFETY: sid is valid and text is an output pointer.
        if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let _text = LocalAllocation(text.cast());
        let mut length = 0;
        // SAFETY: the API returned a NUL-terminated UTF-16 string.
        unsafe {
            while *text.add(length) != 0 {
                length += 1;
            }
        }
        // SAFETY: length was measured within the allocated terminated string.
        let sid = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) });
        let descriptor = wide(Path::new(&format!("O:{sid}D:P(A;;FA;;;{sid})")))?;
        let mut security = ptr::null_mut();
        // SAFETY: descriptor is terminated and output pointer is valid.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                descriptor.as_ptr(),
                SDDL_REVISION_1,
                &mut security,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }

        Ok(Self(LocalAllocation(security)))
    }

    /// Returns non-inheritable creation attributes borrowing this descriptor.
    #[must_use]
    pub(crate) fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: 0,
        }
    }
}

fn validate(file: &File, directory: bool, private: bool) -> io::Result<()> {
    use std::os::windows::fs::MetadataExt;
    // SAFETY: the file owns a live handle; only disk files/directories are accepted.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK {
        return Err(denied());
    }

    let metadata = file.metadata()?;

    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || metadata.is_dir() != directory
        || (!directory && !metadata.is_file())
    {
        return Err(denied());
    }

    if !private {
        return Ok(());
    }

    validate_security(file.as_raw_handle())
}

pub(crate) fn validate_security(handle: std::os::windows::io::RawHandle) -> io::Result<()> {
    let user = user()?;
    // SAFETY: buffer contains TOKEN_USER and remains live for the SID comparisons.
    let sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let mut owner = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    // SAFETY: file is live, all outputs point to initialized slots.
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    let _descriptor = LocalAllocation(descriptor);

    if status != ERROR_SUCCESS {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    // SAFETY: successful GetSecurityInfo returned owner and ACL within descriptor.
    if owner.is_null() || dacl.is_null() || unsafe { EqualSid(owner, sid) } == 0 {
        return Err(denied());
    }
    // SAFETY: dacl points to a valid ACL for the lifetime of descriptor.
    let count = unsafe { (*dacl).AceCount };

    if count == 0 {
        return Err(denied());
    }

    for index in 0..count {
        let mut ace = ptr::null_mut();
        // SAFETY: index is within AceCount, output points to a live slot.
        if unsafe { GetAce(dacl, u32::from(index), &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: GetAce returned a valid ACE header.
        let header = unsafe { &*ace.cast::<ACE_HEADER>() };

        if header.AceType != 0 /* ACCESS_ALLOWED_ACE_TYPE */
            || header.AceSize < mem::size_of::<ACCESS_ALLOWED_ACE>() as u16
        {
            return Err(denied());
        }
        // SAFETY: type and minimum size checked; SidStart is the inline SID.
        let allowed_sid = unsafe {
            ptr::addr_of!((*ace.cast::<ACCESS_ALLOWED_ACE>()).SidStart)
                .cast_mut()
                .cast()
        };
        // SAFETY: Windows supplied a valid ACE SID and token SID.
        if unsafe { EqualSid(allowed_sid, sid) } == 0 {
            return Err(denied());
        }
    }

    Ok(())
}

fn raw_open(
    path: &Path,
    access: u32,
    creation: u32,
    security: Option<&Security>,
    share_delete: bool,
) -> io::Result<File> {
    let path = wide(path)?;
    let attributes = security.map(Security::attributes);
    // Excluding FILE_SHARE_DELETE pins each component against rename/replacement.
    // SAFETY: terminated path and optional attributes are live throughout the call.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | if share_delete { FILE_SHARE_DELETE } else { 0 },
            attributes.as_ref().map_or(ptr::null(), ptr::from_ref),
            creation,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    };

    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: CreateFileW transferred ownership of a valid handle.
    Ok(unsafe { File::from_raw_handle(handle) })
}

fn parents(path: &Path) -> io::Result<Vec<File>> {
    let absolute = std::path::absolute(path)?;
    let mut held = Vec::new();
    let mut current = PathBuf::new();
    let parent = absolute.parent().ok_or_else(denied)?;

    for component in parent.components() {
        current.push(component);

        if matches!(component, std::path::Component::Prefix(_)) || !current.has_root() {
            continue;
        }

        let file = raw_open(&current, FILE_READ_ATTRIBUTES, OPEN_EXISTING, None, false)?;
        validate(&file, true, false)?;
        held.push(file);
    }

    Ok(held)
}

pub(super) fn create(path: &Path) -> io::Result<File> {
    let _parents = parents(path)?;
    let security = Security::current_user()?;
    let file = raw_open(
        path,
        GENERIC_WRITE | READ_CONTROL | FILE_READ_ATTRIBUTES | DELETE,
        CREATE_NEW,
        Some(&security),
        false,
    )?;
    validate(&file, false, true)?;

    Ok(file)
}

pub(super) fn open(path: &Path) -> io::Result<File> {
    let _parents = parents(path)?;
    let file = raw_open(path, GENERIC_READ | READ_CONTROL, OPEN_EXISTING, None, true)?;
    validate(&file, false, true)?;

    Ok(file)
}

pub(super) fn prepare_directory(path: &Path) -> io::Result<()> {
    let _parents = parents(path)?;
    let security = Security::current_user()?;
    let attributes = security.attributes();
    let name = wide(path)?;
    // SAFETY: name and attributes remain live through this synchronous call.
    if unsafe { CreateDirectoryW(name.as_ptr(), &attributes) } == 0 {
        let error = io::Error::last_os_error();

        if error.raw_os_error() != Some(ERROR_ALREADY_EXISTS as i32) {
            return Err(error);
        }
    }

    validate(
        &raw_open(
            path,
            READ_CONTROL | FILE_READ_ATTRIBUTES,
            OPEN_EXISTING,
            None,
            false,
        )?,
        true,
        true,
    )
}

/// Opens source beneath a canonical root, rejecting reparse points on every component.
///
/// # Errors
/// Returns `PermissionDenied` for escapes or reparse points, or filesystem errors.
pub fn open_source(root: &Path, source: &Path) -> io::Result<File> {
    let _root = parents(&root.join(".source-boundary"))?;
    let root = root.canonicalize()?;
    let path = root.join(source);

    if !path.starts_with(&root)
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(denied());
    }

    let _parents = parents(&path)?;
    let file = raw_open(&path, GENERIC_READ, OPEN_EXISTING, None, true)?;
    validate(&file, false, false)?;

    Ok(file)
}

/// Atomically replaces a private JSON/output file with a newly created private file.
///
/// # Errors
/// Returns unsafe-directory, creation, write, or atomic replacement failures.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::{
        io::Write,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or_else(denied)?;
    let _parents = parents(path)?;
    let directory = raw_open(
        parent,
        READ_CONTROL | FILE_READ_ATTRIBUTES,
        OPEN_EXISTING,
        None,
        false,
    )?;
    validate(&directory, true, true)?;

    struct Temporary(PathBuf);

    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    let (_temporary, mut file) = loop {
        let candidate = parent.join(format!(
            ".rake-private-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));

        match create(&candidate) {
            Ok(file) => break (Temporary(candidate), file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };

    file.write_all(bytes)?;
    file.flush()?;
    replace(&file, path)?;

    Ok(())
}

/// Atomically renames an owned private file, preserving readers of the old destination.
/// The file must have been created with [`super::create`].
///
/// # Errors
/// Returns ownership, path validation or Windows rename failures.
pub fn replace(file: &File, path: &Path) -> io::Result<()> {
    validate(file, false, true)?;
    let _parents = parents(path)?;
    let destination = wide(&std::path::absolute(path)?)?;
    let name = &destination[..destination.len() - 1];
    let offset = mem::offset_of!(FILE_RENAME_INFO, FileName);
    let size = offset + mem::size_of_val(name);
    let mut buffer = vec![
        0usize;
        size.max(mem::size_of::<FILE_RENAME_INFO>())
            .div_ceil(mem::size_of::<usize>())
    ];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // FILE_RENAME_REPLACE_IF_EXISTS | FILE_RENAME_POSIX_SEMANTICS (Windows 10+).
    // POSIX semantics preserve active readers of a replaced snapshot.
    // SAFETY: buffer has FILE_RENAME_INFO alignment and enough space for header/name;
    // all fields and inline UTF-16 bytes are initialized within that allocation.
    unsafe {
        (*info).Anonymous.Flags = 0x1 | 0x2;
        (*info).RootDirectory = ptr::null_mut();
        (*info).FileNameLength = mem::size_of_val(name) as u32;
        ptr::copy_nonoverlapping(
            name.as_ptr(),
            ptr::addr_of_mut!((*info).FileName).cast(),
            name.len(),
        );
    }

    // SAFETY: file is live with DELETE access; info covers size initialized bytes.
    if unsafe {
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileRenameInfoEx,
            info.cast(),
            size as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acl_validation_rejects_public_and_null_dacls() {
        let root = std::env::temp_dir().join(format!(
            "rake-acl-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        prepare_directory(&root).unwrap();

        for (index, sddl) in ["D:P(A;;FA;;;WD)", "D:NO_ACCESS_CONTROL"]
            .iter()
            .enumerate()
        {
            let text = wide(Path::new(sddl)).unwrap();
            let mut descriptor = ptr::null_mut();
            // SAFETY: valid terminated SDDL and writable output slot.
            assert_ne!(
                unsafe {
                    ConvertStringSecurityDescriptorToSecurityDescriptorW(
                        text.as_ptr(),
                        SDDL_REVISION_1,
                        &mut descriptor,
                        ptr::null_mut(),
                    )
                },
                0
            );
            let security = Security(LocalAllocation(descriptor));
            let path = root.join(format!("public-{index}"));
            let file = raw_open(&path, GENERIC_WRITE, CREATE_NEW, Some(&security), false).unwrap();
            drop(file);
            assert!(open(&path).is_err(), "accepted {sddl}");
        }

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn atomic_output_and_checkpoints_stay_private_across_replacement() {
        let root = std::env::temp_dir().join(format!(
            "rake-atomic-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        prepare_directory(&root).unwrap();
        let path = root.join("output.json");

        for bytes in [b"first".as_slice(), "é漢字".as_bytes()] {
            // An active reader must not prevent replacement of the next snapshot.
            let previous = open(&path).ok();
            write_atomic(&path, bytes).unwrap();
            if let Some(previous) = previous {
                assert_eq!(std::io::read_to_string(previous).unwrap(), "first");
            }

            let mut read = Vec::new();
            std::io::Read::read_to_end(&mut open(&path).unwrap(), &mut read).unwrap();
            assert_eq!(read, bytes);
        }

        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        assert!(open_source(&root, Path::new("../outside.rs")).is_err());
        drop(
            open_source(&root, Path::new("output.json"))
                .expect("open an in-root source through its canonical Windows path"),
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn directory_junctions_cannot_redirect_private_or_source_opens() {
        let root = std::env::temp_dir().join(format!(
            "rake-junction-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        prepare_directory(&root).unwrap();
        let target = root.join("target");
        prepare_directory(&target).unwrap();
        drop(create(&target.join("source.rs")).unwrap());
        let junction = root.join("junction");
        let status = std::process::Command::new("cmd")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&junction)
            .arg(&target)
            .status()
            .unwrap();
        assert!(status.success());
        assert!(open(&junction.join("source.rs")).is_err());
        assert!(create(&junction.join("new")).is_err());
        assert!(prepare_directory(&junction).is_err());
        assert!(open_source(&root, Path::new("junction/source.rs")).is_err());
        std::fs::remove_dir(&junction).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
}
