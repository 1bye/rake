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

    // A fresh apartment keeps COM initialization and interface destruction on
    // one thread, independent of the caller's runtime or apartment model.
    std::thread::spawn({
        let target = target.to_owned();

        move || -> windows::core::Result<()> {
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
                let shell: IShellWindows =
                    CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)?;
                let empty = VARIANT::default();
                let mut hwnd = 0;
                let desktop = shell.FindWindowSW(
                    &empty,
                    &empty,
                    SWC_DESKTOP,
                    &mut hwnd,
                    SWFO_NEEDDISPATCH,
                )?;
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
    })
    .join()
    .map_err(|_| "Windows desktop source opener failed")??;

    Ok(())
}
