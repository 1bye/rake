use crate::{
    Anchor, CoordinateSpace, Error, EventKind, Input, Point, Rect, Registration, Request, Tray,
    host::Sink,
};
use std::{
    cell::Cell,
    ffi::c_void,
    mem::size_of,
    ptr::{null, null_mut},
    rc::Rc,
};
use windows_sys::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::GUID,
};

const NIN_KEYSELECT: u32 = NIN_SELECT | NINF_KEY;
const CALLBACK: u32 = WM_APP + 0x321;
const CLASS: &[u16] = &[
    82, 97, 107, 101, 84, 114, 97, 121, 79, 119, 110, 101, 114, 0,
];

struct WindowState {
    sink: Sink,
    data: Cell<NOTIFYICONDATAW>,
    taskbar_created: u32,
    removed: Cell<bool>,
    revision: Cell<u64>,
}

// SAFETY: This procedure only borrows WindowState while its owning Item keeps the
// hidden HWND alive. No application callback runs here; only owned events are queued.
unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_NCCREATE {
        // SAFETY: Windows supplies CREATESTRUCTW during WM_NCCREATE; lpCreateParams is our live Box.
        unsafe {
            let create = &*(lparam as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
    }
    // SAFETY: User data is initialized at creation and cleared before the Box is released.
    let state = unsafe { (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowState).as_ref() };

    if let Some(state) = state {
        if message == state.taskbar_created && !state.removed.get() {
            let mut data = state.data.get();
            // SAFETY: Correctly initialized structure and retained HICON belong to this HWND.
            unsafe {
                if Shell_NotifyIconW(NIM_ADD, &data) != 0 {
                    data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                    Shell_NotifyIconW(NIM_SETVERSION, &data);
                    state
                        .sink
                        .send(EventKind::HostChanged(true), None, Input::default());
                } else {
                    state
                        .sink
                        .send(EventKind::HostChanged(false), None, Input::default());
                }
            }

            return 0;
        }
        if message == CALLBACK && !state.removed.get() {
            let code = (lparam as u32) & 0xffff;
            let kind = match code {
                NIN_SELECT | NIN_KEYSELECT => Some(EventKind::Activate),
                WM_CONTEXTMENU => Some(EventKind::ContextMenu),
                _ => None,
            };

            if let Some(kind) = kind {
                let x = (wparam as u16) as i16;
                let y = ((wparam >> 16) as u16) as i16;
                let anchor = Anchor {
                    rect: None,
                    point: Some(Point {
                        x: x.into(),
                        y: y.into(),
                    }),
                    space: CoordinateSpace::PhysicalScreen,
                };
                state.sink.send(
                    kind,
                    Some(anchor),
                    Input {
                        keyboard: Some(code == NIN_KEYSELECT),
                        ..Input::default()
                    },
                );
            }

            return 0;
        }
    }

    // SAFETY: All unhandled messages retain their original native parameters.
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn failure(operation: &'static str) -> Error {
    crate::native(operation, std::io::Error::last_os_error())
}

pub(crate) struct Item {
    hwnd: HWND,
    state: Box<WindowState>,
}

impl Item {
    pub(crate) fn check_thread() -> Result<(), Error> {
        Ok(())
    }
    pub(crate) fn create(application: &str, spec: &Tray, sink: Sink) -> Request<Self> {
        crate::ready(Self::new(application, spec, sink))
    }

    fn new(application: &str, spec: &Tray, sink: Sink) -> Result<Self, Error> {
        // SAFETY: Registers a static class/procedure; re-registering an existing class is harmless.
        let instance = unsafe { GetModuleHandleW(null()) };
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: CLASS.as_ptr(),
            ..Default::default()
        };
        // SAFETY: All class pointers live for process lifetime.
        unsafe {
            RegisterClassW(&class);
        }
        let identity = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            format!("{application}/{}", spec.id).as_bytes(),
        );
        let data = NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_GUID | NIF_ICON | NIF_TIP | NIF_SHOWTIP | NIF_STATE,
            uCallbackMessage: CALLBACK,
            guidItem: GUID::from_u128(identity.as_u128()),
            dwStateMask: NIS_HIDDEN,
            ..Default::default()
        };
        // SAFETY: String is terminated and remains alive for this synchronous call.
        let taskbar_created = unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) };
        let mut state = Box::new(WindowState {
            sink,
            data: Cell::new(data),
            taskbar_created,
            removed: Cell::new(false),
            revision: Cell::new(0),
        });
        // A hidden top-level window receives TaskbarCreated broadcasts; HWND_MESSAGE does not.
        // SAFETY: Box remains at a stable address until DestroyWindow completes in Item::drop.
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                CLASS.as_ptr(),
                CLASS.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                (&mut *state as *mut WindowState).cast::<c_void>(),
            )
        };

        if hwnd.is_null() {
            return Err(failure("create tray callback window"));
        }

        let item = Self { hwnd, state };
        let mut data = item.state.data.get();
        data.hWnd = hwnd;
        item.state.data.set(data);
        item.prepare(spec)?;
        let mut data = item.state.data.get();
        // SAFETY: Native structure and icon are initialized and owned by item.
        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) } == 0 {
            return Err(failure("add tray icon"));
        }

        data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        // SAFETY: Same stable native identity, version-4 union is initialized.
        if unsafe { Shell_NotifyIconW(NIM_SETVERSION, &data) } == 0 {
            return Err(failure("negotiate tray version"));
        }

        Ok(item)
    }

    fn prepare(&self, spec: &Tray) -> Result<(), Error> {
        let icon = spec.image.as_ref().map(make_icon).transpose()?;
        let mut data = self.state.data.get();
        let old = data.hIcon;
        data.hIcon = icon.unwrap_or(null_mut());
        data.szTip.fill(0);

        for (slot, ch) in data.szTip.iter_mut().zip(spec.tooltip.encode_utf16()) {
            *slot = ch;
        }

        data.dwState = if spec.visible { 0 } else { NIS_HIDDEN };
        self.state.data.set(data);

        if !old.is_null() {
            // SAFETY: This HICON was created by make_icon and is no longer the desired resource.
            unsafe {
                DestroyIcon(old);
            }
        }

        Ok(())
    }

    pub(crate) fn apply(self: &Rc<Self>, revision: u64, spec: &Tray) -> Request<()> {
        let item = self.clone();
        let spec = spec.clone();
        Box::pin(async move {
            if item.state.removed.get() {
                return Err(Error::Removed);
            }
            if item.state.revision.get() > revision {
                return Ok(());
            }
            item.prepare(&spec)?;
            let data = item.state.data.get();
            // SAFETY: Correct live identity and owned resources remain valid during the call.
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } == 0 {
                return Err(failure("update tray icon"));
            }
            item.state.revision.set(revision);
            Ok(())
        })
    }
    pub(crate) fn registration(&self) -> Registration {
        Registration::Registered
    }
    pub(crate) fn anchor(&self) -> Result<Option<Anchor>, Error> {
        let data = self.state.data.get();
        let id = NOTIFYICONIDENTIFIER {
            cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: self.hwnd,
            uID: data.uID,
            guidItem: data.guidItem,
        };
        let mut rect = RECT::default();
        // SAFETY: Output storage and native identity are valid for the duration of this call.
        if unsafe { Shell_NotifyIconGetRect(&id, &mut rect) } < 0 {
            return Ok(None);
        }

        Ok(Some(Anchor {
            rect: Some(Rect {
                origin: Point {
                    x: rect.left.into(),
                    y: rect.top.into(),
                },
                width: (rect.right - rect.left).into(),
                height: (rect.bottom - rect.top).into(),
            }),
            point: None,
            space: CoordinateSpace::PhysicalScreen,
        }))
    }
    pub(crate) fn shutdown(&self) -> Request<()> {
        if !self.state.removed.replace(true) {
            // SAFETY: Same native identity; repeated deletion is prevented.
            unsafe {
                Shell_NotifyIconW(NIM_DELETE, &self.state.data.get());
            }
        }

        crate::ready(Ok(()))
    }
}

impl Drop for Item {
    fn drop(&mut self) {
        drop(self.shutdown());
        // SAFETY: The HWND and Box are exclusively owned; no callback can access the Box after destroy.
        unsafe {
            DestroyWindow(self.hwnd);
            let icon = self.state.data.get().hIcon;

            if !icon.is_null() {
                DestroyIcon(icon);
            }
        }
    }
}

fn make_icon(image: &crate::Image) -> Result<HICON, Error> {
    let rep = image
        .representations
        .iter()
        .min_by_key(|r| r.width.abs_diff(32))
        .ok_or(Error::Unsupported(crate::Feature::NamedImage))?;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: rep.width as i32,
            biHeight: -(rep.height as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits = null_mut();
    // SAFETY: Initialized BITMAPINFO and valid output pointer; raster sizes are bounded before here.
    let color =
        unsafe { CreateDIBSection(null_mut(), &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0) };

    if color.is_null() {
        return Err(failure("create tray bitmap"));
    }

    // SAFETY: DIBSection owns width*height*4 bytes; converting RGBA into premultiplied BGRA.
    unsafe {
        let output = std::slice::from_raw_parts_mut(bits.cast::<u8>(), rep.rgba.len());

        for (rgba, bgra) in rep.rgba.chunks_exact(4).zip(output.chunks_exact_mut(4)) {
            let a = u16::from(rgba[3]);
            bgra.copy_from_slice(&[
                (u16::from(rgba[2]) * a / 255) as u8,
                (u16::from(rgba[1]) * a / 255) as u8,
                (u16::from(rgba[0]) * a / 255) as u8,
                rgba[3],
            ]);
        }
    }
    let mask_bytes = vec![0u8; rep.width.div_ceil(16) as usize * 2 * rep.height as usize];
    // SAFETY: Valid dimensions and a word-aligned monochrome bitmap buffer.
    let mask = unsafe {
        CreateBitmap(
            rep.width as i32,
            rep.height as i32,
            1,
            1,
            mask_bytes.as_ptr().cast(),
        )
    };

    if mask.is_null() {
        // SAFETY: The color bitmap was created above and is not attached to an HICON.
        unsafe {
            DeleteObject(color);
        }
        return Err(failure("create tray mask"));
    }
    let info = ICONINFO {
        fIcon: 1,
        hbmMask: mask,
        hbmColor: color,
        ..Default::default()
    };
    // SAFETY: CreateIconIndirect copies the bitmap data before both temporary bitmaps are destroyed.
    let icon = unsafe { CreateIconIndirect(&info) };
    // SAFETY: Both temporary GDI handles belong to this function.
    unsafe {
        DeleteObject(mask);
        DeleteObject(color);
    }

    if icon.is_null() {
        return Err(failure("create tray icon"));
    }

    Ok(icon)
}
