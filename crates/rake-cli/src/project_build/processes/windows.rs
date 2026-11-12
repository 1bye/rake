//! Assign suspended tools to an owned kill-on-close Job before any child code runs.

use super::ProcessGroup;
use std::{
    fs::File,
    io,
    mem::{size_of, zeroed},
    os::windows::{
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
        process::CommandExt,
    },
    process::{Command, Stdio},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Pipes::CreatePipe,
        Threading::{CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
    },
};

pub(super) struct Job(OwnedHandle);

fn owned(handle: HANDLE) -> io::Result<OwnedHandle> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: each caller transfers a newly created, valid, uniquely owned HANDLE.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

impl Job {
    pub(super) fn terminate(&self) {
        // SAFETY: the handle remains owned by this Job; only its assigned tools are affected.
        unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) };
    }
}

pub(super) fn spawn(command: &mut Command) -> super::super::Result<ProcessGroup> {
    // SAFETY: null attributes/name create a new unnamed, non-inheritable Job.
    let job = Job(owned(unsafe { CreateJobObjectW(null(), null()) })?);
    // SAFETY: all-zero fields are valid defaults for this C information structure.
    let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { zeroed() };
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

    // SAFETY: limits is live and has the size required by the selected information class.
    if unsafe {
        SetInformationJobObject(
            job.0.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    } == 0
    {
        return Err(io::Error::last_os_error().into());
    }

    let mut child = command.creation_flags(CREATE_SUSPENDED).spawn()?;
    let result = (|| -> io::Result<()> {
        // SAFETY: child and job are valid owned handles. The suspended child has not run.
        if unsafe { AssignProcessToJobObject(job.0.as_raw_handle(), child.as_raw_handle()) } == 0 {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: snapshot creation returns a new owned handle or INVALID_HANDLE_VALUE.
        let snapshot = owned(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) })?;
        // SAFETY: zero fields plus the documented size initialize a THREADENTRY32 buffer.
        let mut entry: THREADENTRY32 = unsafe { zeroed() };
        entry.dwSize = size_of::<THREADENTRY32>() as u32;
        // SAFETY: snapshot is live and entry is a correctly sized output buffer.
        let mut found = unsafe { Thread32First(snapshot.as_raw_handle(), &mut entry) } != 0;

        while found {
            if entry.th32OwnerProcessID == child.id() {
                // SAFETY: opening the suspended child's thread requests only resume access.
                let thread =
                    owned(unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) })?;
                // SAFETY: this is the initial thread of our CREATE_SUSPENDED child, assigned above.
                if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
                    return Err(io::Error::last_os_error());
                }

                return Ok(());
            }

            // SAFETY: snapshot and the same correctly sized entry buffer remain live.
            found = unsafe { Thread32Next(snapshot.as_raw_handle(), &mut entry) } != 0;
        }

        Err(io::Error::other("suspended tool thread was not found"))
    })();

    if let Err(error) = result {
        job.terminate();
        // Assignment might have failed before the child entered the Job.
        let _ = child.kill();
        let _ = child.wait();

        return Err(error.into());
    }

    Ok(ProcessGroup(child, job))
}

pub(super) fn pipe(merge: bool) -> io::Result<(File, Stdio, Option<Stdio>)> {
    let mut reader = null_mut();
    let mut writer = null_mut();

    // SAFETY: pointers are writable HANDLE outputs. Rust Command duplicates only
    // its configured standard handles as inheritable; these originals stay private.
    if unsafe { CreatePipe(&mut reader, &mut writer, null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }

    let reader = File::from(owned(reader)?);
    let writer = File::from(owned(writer)?);
    let stdout = Stdio::from(writer.try_clone()?);
    let stderr = merge.then(|| Stdio::from(writer));

    Ok((reader, stdout, stderr))
}

#[cfg(test)]
mod tests {
    //! Real Windows Jobs, anonymous pipes and descendant cleanup; no shell tool installs.
    use super::*;
    use std::{
        sync::atomic::AtomicBool,
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::STILL_ACTIVE,
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };

    #[test]
    fn captures_merged_diagnostics_and_preserves_failure_status() {
        let cancelled = AtomicBool::new(false);
        let processes = super::super::Processes::new(&cancelled);
        let output = processes
            .capture(
                Command::new("cmd").args(["/D", "/C", "echo stdout & echo stderr 1>&2"]),
                super::super::Capture::Diagnostic,
            )
            .unwrap();
        assert!(
            output.contains("stdout") && output.contains("stderr"),
            "{output}"
        );
        let error = processes
            .capture(
                Command::new("cmd").args(["/D", "/C", "exit 7"]),
                super::super::Capture::Diagnostic,
            )
            .unwrap_err();
        assert!(error.to_string().contains("non-zero exit status 7"));
    }

    #[test]
    fn job_drop_stops_descendants_and_metadata_stays_bounded() {
        let root = tempfile::tempdir().unwrap();
        let mut command = fixture(root.path(), "parent");
        let group = spawn(&mut command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        let path = root.path().join("descendant");
        let pid = loop {
            if let Some(pid) = std::fs::read_to_string(&path)
                .ok()
                .and_then(|text| text.parse::<u32>().ok())
            {
                break pid;
            }
            assert!(Instant::now() < deadline, "descendant did not start");
            std::thread::sleep(Duration::from_millis(10));
        };
        // SAFETY: open only our fixture descendant for exit-status observation.
        let child =
            owned(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) }).unwrap();
        drop(group);
        loop {
            let mut code = 0;
            // SAFETY: child owns a valid process handle and code is a writable u32.
            assert_ne!(
                unsafe { GetExitCodeProcess(child.as_raw_handle(), &mut code) },
                0
            );
            if code != STILL_ACTIVE as u32 {
                break;
            }
            assert!(Instant::now() < deadline, "Job descendant survived cleanup");
            std::thread::sleep(Duration::from_millis(10));
        }
        let cancelled = AtomicBool::new(false);
        let error = super::super::Processes::new(&cancelled)
            .capture(
                &mut fixture(root.path(), "output"),
                super::super::Capture::Metadata,
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "tool metadata exceeds 32 MiB");
    }

    fn fixture(root: &std::path::Path, mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--ignored",
                "--exact",
                "project_build::processes::windows::tests::job_fixture",
                "--nocapture",
            ])
            .env("RAKE_TEST_JOB_MODE", mode)
            .env("RAKE_TEST_JOB_ROOT", root);
        command
    }

    #[test]
    #[ignore = "subprocess entry for Windows Job ownership tests"]
    fn job_fixture() {
        let mode = std::env::var("RAKE_TEST_JOB_MODE").expect("fixture mode");
        let root =
            std::path::PathBuf::from(std::env::var_os("RAKE_TEST_JOB_ROOT").expect("fixture root"));
        match mode.as_str() {
            "parent" => {
                fixture(&root, "descendant")
                    .spawn()
                    .unwrap()
                    .wait()
                    .unwrap();
            }
            "descendant" => {
                std::fs::write(root.join("descendant"), std::process::id().to_string()).unwrap();
                std::thread::sleep(Duration::from_secs(300));
            }
            "output" => {
                use std::io::Write;
                let mut output = std::io::stdout().lock();
                for _ in 0..33 {
                    let _ = output.write_all(&vec![0; 1024 * 1024]);
                }
            }
            _ => panic!("unknown fixture mode"),
        }
    }
}
