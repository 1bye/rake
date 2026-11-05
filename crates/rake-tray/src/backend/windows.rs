use crate::{
    Anchor, CoordinateSpace, Error, EventKind, Input, Point, Rect, Registration, Request, Tray,
    host::Sink,
};
use std::{
    cell::{Cell, RefCell},
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
        UI::{
            HiDpi::{GetDpiForMonitor, GetDpiForWindow, GetSystemMetricsForDpi, MDT_EFFECTIVE_DPI},
            Shell::*,
            WindowsAndMessaging::*,
        },
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
    image: RefCell<Option<crate::Image>>,
    balloon_icon: Cell<HICON>,
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
        if matches!(message, WM_DPICHANGED | WM_DISPLAYCHANGE | WM_SETTINGCHANGE)
            && !state.removed.get()
        {
            let image = state.image.borrow().clone();
            if let Some(image) = image
                && let Ok(icon) = make_icon(&image, hwnd)
            {
                let mut data = state.data.get();
                let old = data.hIcon;
                data.hIcon = icon;
                // SAFETY: Copies the new icon before releasing the old cache handle.
                unsafe {
                    Shell_NotifyIconW(NIM_MODIFY, &data);
                    if !old.is_null() {
                        DestroyIcon(old);
                    }
                }
                state.data.set(data);
            }
        }
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
                NIN_BALLOONSHOW => Some(EventKind::Balloon(crate::BalloonEvent::Shown)),
                NIN_BALLOONHIDE => Some(EventKind::Balloon(crate::BalloonEvent::Hidden)),
                NIN_BALLOONTIMEOUT => Some(EventKind::Balloon(crate::BalloonEvent::TimedOut)),
                NIN_BALLOONUSERCLICK => Some(EventKind::Balloon(crate::BalloonEvent::Selected)),
                NIN_POPUPOPEN => Some(EventKind::TooltipRequested),
                NIN_POPUPCLOSE => Some(EventKind::TooltipDismissed),
                WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
                    Some(EventKind::Pointer(crate::PointerAction::Down))
                }
                WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => {
                    Some(EventKind::Pointer(crate::PointerAction::Up))
                }
                WM_LBUTTONDBLCLK | WM_RBUTTONDBLCLK | WM_MBUTTONDBLCLK => {
                    Some(EventKind::Pointer(crate::PointerAction::DoubleClick))
                }
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
                // Version-4 coordinates are documented for select/keyboard select;
                // WM_CONTEXTMENU does not promise a meaningful packed position.
                let anchor =
                    matches!(code, NIN_SELECT | NIN_KEYSELECT | NIN_POPUPOPEN).then_some(anchor);
                // SAFETY: Reads the timestamp of this thread's current native message.
                let timestamp = unsafe { GetMessageTime() } as u32 as u64;
                state.sink.send(
                    kind,
                    anchor,
                    Input {
                        timestamp: Some(std::time::Duration::from_millis(timestamp)),
                        button: match code {
                            WM_LBUTTONDOWN | WM_LBUTTONUP | WM_LBUTTONDBLCLK => {
                                Some(crate::Button::Primary)
                            }
                            WM_RBUTTONDOWN | WM_RBUTTONUP | WM_RBUTTONDBLCLK => {
                                Some(crate::Button::Secondary)
                            }
                            WM_MBUTTONDOWN | WM_MBUTTONUP | WM_MBUTTONDBLCLK => {
                                Some(crate::Button::Middle)
                            }
                            _ => None,
                        },
                        keyboard: match code {
                            NIN_KEYSELECT => Some(true),
                            NIN_SELECT => Some(false),
                            _ => None,
                        },
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
    menu: RefCell<Option<Rc<super::windows_menu::NativeMenu>>>,
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
        let derived_identity = uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            format!("{application}/{}", spec.id).as_bytes(),
        );
        let identity = spec
            .windows
            .as_ref()
            .and_then(|w| w.guid)
            .map(uuid::Uuid::from_bytes)
            .unwrap_or(derived_identity);
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
            image: RefCell::new(None),
            balloon_icon: Cell::new(null_mut()),
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

        let item = Self {
            hwnd,
            state,
            menu: RefCell::new(None),
        };
        let mut data = item.state.data.get();
        data.hWnd = hwnd;
        item.state.data.set(data);
        item.prepare(0, spec)?;
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

    fn prepare(&self, revision: u64, spec: &Tray) -> Result<(), Error> {
        let menu = spec
            .menu
            .as_ref()
            .map(|menu| {
                super::windows_menu::NativeMenu::new(menu, self.state.sink.menu(revision))
                    .map(Rc::new)
            })
            .transpose()?;
        let icon = spec
            .image
            .as_ref()
            .map(|image| make_icon(image, self.hwnd))
            .transpose()?;
        *self.menu.borrow_mut() = menu;
        *self.state.image.borrow_mut() = spec.image.clone();
        let mut data = self.state.data.get();
        let old = data.hIcon;
        data.hIcon = icon.unwrap_or(null_mut());
        data.szTip.fill(0);

        for (slot, ch) in data.szTip.iter_mut().zip(spec.tooltip.encode_utf16()) {
            *slot = ch;
        }

        if spec
            .windows
            .as_ref()
            .is_some_and(|w| w.tooltip == crate::windows::TooltipMode::Events)
        {
            data.uFlags &= !NIF_SHOWTIP;
        } else {
            data.uFlags |= NIF_SHOWTIP;
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
            item.prepare(revision, &spec)?;
            let data = item.state.data.get();
            // SAFETY: Correct live identity and owned resources remain valid during the call.
            if unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } == 0 {
                return Err(failure("update tray icon"));
            }
            item.state.revision.set(revision);
            Ok(())
        })
    }
    pub(crate) fn show_menu(&self) -> Result<(), Error> {
        let menu = self
            .menu
            .borrow()
            .clone()
            .ok_or(Error::InvalidInput("item has no menu".into()))?;
        let anchor = self.anchor()?.and_then(|a| a.rect);
        let mut position = POINT::default();
        if let Some(rect) = anchor {
            position.x = rect.origin.x as i32;
            position.y = (rect.origin.y + rect.height) as i32;
        } else {
            // SAFETY: Valid writable output point.
            unsafe {
                GetCursorPos(&mut position);
            }
        }
        menu.show(self.hwnd, position.x, position.y)?;
        let data = self.state.data.get();
        // SAFETY: Restore notification-area keyboard focus using our live identity.
        unsafe {
            Shell_NotifyIconW(NIM_SETFOCUS, &data);
        }
        Ok(())
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
    pub(crate) fn legacy_balloon(&self, balloon: crate::windows::LegacyBalloon) -> Request<()> {
        use crate::windows::BalloonIcon;
        let result = (|| {
            if self.state.removed.get() {
                return Err(Error::Removed);
            }
            let custom = match &balloon.icon {
                BalloonIcon::Custom(image) => Some(make_icon(image, self.hwnd)?),
                _ => None,
            };
            let mut data = self.state.data.get();
            data.uFlags = NIF_GUID | NIF_INFO | if balloon.realtime { NIF_REALTIME } else { 0 };
            data.szInfo.fill(0);
            data.szInfoTitle.fill(0);
            for (slot, ch) in data.szInfo.iter_mut().zip(balloon.body.encode_utf16()) {
                *slot = ch;
            }
            for (slot, ch) in data
                .szInfoTitle
                .iter_mut()
                .zip(balloon.title.encode_utf16())
            {
                *slot = ch;
            }
            data.dwInfoFlags = match balloon.icon {
                BalloonIcon::None => NIIF_NONE,
                BalloonIcon::Information => NIIF_INFO,
                BalloonIcon::Warning => NIIF_WARNING,
                BalloonIcon::Error => NIIF_ERROR,
                BalloonIcon::Custom(_) => NIIF_USER,
            } | if balloon.silent { NIIF_NOSOUND } else { 0 }
                | if balloon.large_icon {
                    NIIF_LARGE_ICON
                } else {
                    0
                }
                | if balloon.respect_quiet_time {
                    NIIF_RESPECT_QUIET_TIME
                } else {
                    0
                };
            data.hBalloonIcon = custom.unwrap_or(null_mut());
            // SAFETY: Valid fixed buffers and owned custom HICON remain live during Shell copy.
            let accepted = unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) } != 0;
            if !accepted {
                if let Some(icon) = custom {
                    /* SAFETY: New, unattached icon owned here. */
                    unsafe {
                        DestroyIcon(icon);
                    }
                }
                return Err(failure("send legacy tray balloon"));
            }
            let old = self
                .state
                .balloon_icon
                .replace(custom.unwrap_or(null_mut()));
            if !old.is_null() {
                /* SAFETY: Previous balloon image is no longer the desired resource. */
                unsafe {
                    DestroyIcon(old);
                }
            }
            Ok(())
        })();
        crate::ready(result)
    }

    pub(crate) fn popup_anchor(&self) -> Result<Option<crate::PopupAnchor>, Error> {
        let Some(icon) = self.anchor()?.and_then(|anchor| anchor.rect) else {
            return Ok(None);
        };
        let rect = RECT {
            left: icon.origin.x as i32,
            top: icon.origin.y as i32,
            right: (icon.origin.x + icon.width) as i32,
            bottom: (icon.origin.y + icon.height) as i32,
        };
        // SAFETY: Valid rectangle/output storage and live monitor handle.
        unsafe {
            let monitor = MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return Err(failure("query popup monitor"));
            }
            let (mut x, mut y) = (0, 0);
            let result = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y);
            if result < 0 || x == 0 {
                return Err(Error::HostUnavailable);
            }
            let scale = f64::from(x) / 96.0;
            let convert = |rect: RECT| Rect {
                origin: Point {
                    x: f64::from(rect.left) / scale,
                    y: f64::from(rect.top) / scale,
                },
                width: f64::from(rect.right - rect.left) / scale,
                height: f64::from(rect.bottom - rect.top) / scale,
            };
            Ok(Some(crate::PopupAnchor {
                display_id: monitor as usize as u64,
                icon: convert(rect),
                work_area: convert(info.rcWork),
            }))
        }
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
            let balloon = self.state.balloon_icon.get();
            if !balloon.is_null() {
                DestroyIcon(balloon);
            }
            let icon = self.state.data.get().hIcon;

            if !icon.is_null() {
                DestroyIcon(icon);
            }
        }
    }
}

fn make_icon(image: &crate::Image, hwnd: HWND) -> Result<HICON, Error> {
    // SAFETY: hwnd belongs to this item and remains live throughout image preparation.
    let target =
        unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForWindow(hwnd).max(96)) }.max(1) as u32;
    let rep = image
        .representations
        .iter()
        .min_by_key(|r| r.width.abs_diff(target))
        .ok_or(Error::Unsupported(crate::Feature::NamedImage))?;
    let color = bitmap(rep)?;
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

pub(super) fn bitmap(rep: &crate::ImageRepresentation) -> Result<HBITMAP, Error> {
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

        for (rgba, bgra) in rep
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .zip(output.as_chunks_mut::<4>().0.iter_mut())
        {
            let a = u16::from(rgba[3]);
            bgra.copy_from_slice(&[
                (u16::from(rgba[2]) * a / 255) as u8,
                (u16::from(rgba[1]) * a / 255) as u8,
                (u16::from(rgba[0]) * a / 255) as u8,
                rgba[3],
            ]);
        }
    }
    Ok(color)
}
