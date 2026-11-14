//! Ask the existing desktop Explorer to open files outside Rake's owned Jobs.

use windows::{
    Win32::{
        System::{
            Com::{
                CLSCTX_LOCAL_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoUninitialize, IServiceProvider,
            },
            Variant::VARIANT,
        },
        UI::Shell::{
            IShellBrowser, IShellDispatch2, IShellFolderViewDual, IShellWindows,
            SID_STopLevelBrowser, SVGIO_BACKGROUND, SWC_DESKTOP, SWFO_NEEDDISPATCH, ShellWindows,
        },
    },
    core::{BSTR, Interface},
};

pub(crate) fn open(target: &str) -> super::Result<()> {
    if target.contains('\0') {
        return Err("Source opener target contains NUL".into());
    }

    let deadline = rake_client::current_deadline()
        .unwrap_or_else(|| std::time::Instant::now() + std::time::Duration::from_secs(10));
    let mut command = std::process::Command::new(std::env::current_exe()?);

    command
        .args(["__shell-open", target])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    crate::project_build::shell_helper(&mut command, deadline)
}

pub(crate) fn helper(arguments: &[String]) -> super::Result<()> {
    let [target] = arguments else {
        return Err("Windows shell helper requires exactly one target".into());
    };

    if target.contains('\0') {
        return Err("Source opener target contains NUL".into());
    }

    explorer(target).map_err(|error| error.to_string().into())
}

fn explorer(target: &str) -> windows::core::Result<()> {
    struct Apartment;

    impl Drop for Apartment {
        fn drop(&mut self) {
            // SAFETY: this thread successfully initialized COM below;
            // all its interface variables are dropped before this guard.
            unsafe { CoUninitialize() };
        }
    }

    // SAFETY: this fresh thread has no existing COM apartment.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()? };
    let _apartment = Apartment;

    // SAFETY: typed interfaces retain their COM references and all
    // pointers refer to live variants/output storage in this apartment.
    // FindWindowSW selects the existing desktop, not a new Shell object
    // hosted in our process/Job. Failure has no local-launch fallback.
    unsafe {
        let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)?;
        let empty = VARIANT::default();
        let mut hwnd = 0;
        let desktop =
            shell.FindWindowSW(&empty, &empty, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)?;
        let provider: IServiceProvider = desktop.cast()?;
        let browser: IShellBrowser = provider.QueryService(&SID_STopLevelBrowser)?;
        let view = browser.QueryActiveShellView()?;
        let folder: IShellFolderViewDual = view.GetItemObject(SVGIO_BACKGROUND)?;
        let dispatch: IShellDispatch2 = folder.Application()?.cast()?;

        dispatch.ShellExecute(
            &BSTR::from(target),
            &empty,
            &empty,
            &VARIANT::from("open"),
            &VARIANT::from(1i32),
        )
    }
}
