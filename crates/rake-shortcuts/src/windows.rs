//! Message-only windows share the caller's UI message pump.
use super::Error;
use futures::channel::mpsc;
use rake_tray::Accelerator;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{
            MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
            UnregisterHotKey, VkKeyScanW,
        },
        WindowsAndMessaging::*,
    },
};

pub(super) struct Registration {
    window: HWND,
    sender: Box<mpsc::Sender<()>>,
    registered: bool,
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_HOTKEY {
        // SAFETY: The pointer is installed only after window creation, refers to
        // a stable box, and is cleared before that box or the window is destroyed.
        unsafe {
            let sender = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut mpsc::Sender<()>;

            if let Some(sender) = sender.as_mut() {
                let _ = sender.try_send(());
            }
        }

        return 0;
    }

    // SAFETY: Unhandled messages retain the arguments supplied by the OS.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

pub(super) async fn register(
    accelerator: Accelerator,
    sender: mpsc::Sender<()>,
) -> Result<Registration, Error> {
    let key = accelerator
        .key
        .chars()
        .next()
        .ok_or_else(|| Error::InvalidInput("missing key".into()))?;

    if !key.is_ascii() {
        return Err(Error::InvalidInput(
            "Windows global shortcuts require an ASCII key".into(),
        ));
    }

    // SAFETY: VkKeyScanW accepts any UTF-16 code unit and returns -1 if unmappable.
    let mapped = unsafe { VkKeyScanW(key as u16) };

    if mapped == -1 {
        return Err(Error::InvalidInput(
            "key is absent from the current keyboard layout".into(),
        ));
    }

    let modifiers = MOD_NOREPEAT
        | if accelerator.control || mapped & 0x200 != 0 {
            MOD_CONTROL
        } else {
            0
        }
        | if accelerator.alt || mapped & 0x400 != 0 {
            MOD_ALT
        } else {
            0
        }
        | if accelerator.shift || mapped & 0x100 != 0 {
            MOD_SHIFT
        } else {
            0
        }
        | if accelerator.super_key { MOD_WIN } else { 0 };
    let class: Vec<u16> = "RakeGlobalShortcut\0".encode_utf16().collect();
    // SAFETY: The class and window names are NUL-terminated. The window is owned
    // by this UI thread; it never stores a pointer to stack data.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let definition = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            ..Default::default()
        };

        if RegisterClassW(&definition) == 0
            && std::io::Error::last_os_error().raw_os_error() != Some(1410)
        {
            return Err(last_error());
        }

        let window = CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );

        if window.is_null() {
            return Err(last_error());
        }

        let mut registration = Registration {
            window,
            sender: Box::new(sender),
            registered: false,
        };
        SetWindowLongPtrW(
            window,
            GWLP_USERDATA,
            (&mut *registration.sender as *mut mpsc::Sender<()>) as isize,
        );

        if RegisterHotKey(window, 1, modifiers, (mapped & 0xff) as u32) == 0 {
            let error = std::io::Error::last_os_error();

            return Err(if error.raw_os_error() == Some(1409) {
                Error::Conflict
            } else {
                Error::Native(error.to_string())
            });
        }

        registration.registered = true;

        Ok(registration)
    }
}

impl Registration {
    pub(super) async fn unregister(&mut self) -> Result<(), Error> {
        self.release()
    }

    fn release(&mut self) -> Result<(), Error> {
        // SAFETY: The window and hotkey belong to this registration and thread.
        unsafe {
            if self.registered {
                if UnregisterHotKey(self.window, 1) == 0 {
                    return Err(last_error());
                }

                self.registered = false;
            }

            if !self.window.is_null() {
                SetWindowLongPtrW(self.window, GWLP_USERDATA, 0);

                if DestroyWindow(self.window) == 0 {
                    return Err(last_error());
                }

                self.window = std::ptr::null_mut();
            }
        }

        Ok(())
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

fn last_error() -> Error {
    Error::Native(std::io::Error::last_os_error().to_string())
}
