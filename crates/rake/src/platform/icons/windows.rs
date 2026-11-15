//! Shell icon lookup and bounded DIB rasterization on blocking workers.

use super::Identity;
use std::{os::windows::ffi::OsStrExt, path::Path};
use windows_sys::Win32::{
    Graphics::Gdi::*,
    System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
    UI::{Shell::*, WindowsAndMessaging::*},
};

struct Apartment;

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: Created only after this thread successfully initialized COM.
        unsafe {
            CoUninitialize();
        }
    }
}

pub(super) fn load(path: &Path, pixels: u32) -> Option<(Identity, Vec<u8>)> {
    // SAFETY: This blocking worker initializes and balances its own COM apartment.
    let initialized = unsafe { CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32) };
    let _apartment = if initialized >= 0 {
        Some(Apartment)
    } else {
        None
    };

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();

    if wide[..wide.len() - 1].contains(&0) {
        return None;
    }

    let extension = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    let identity = if path.is_symlink()
        || matches!(extension.as_str(), "exe" | "lnk" | "ico")
        || path.join("desktop.ini").exists()
    {
        Identity::Custom(path.to_path_buf())
    } else if path.is_dir() {
        Identity::Folder
    } else {
        Identity::FileType(extension)
    };

    // SAFETY: All handles are used only in this scope and released below. The
    // terminated UTF-16 path and initialized structure have the documented sizes.
    unsafe {
        let mut info: SHFILEINFOW = std::mem::zeroed();

        if SHGetFileInfoW(
            wide.as_ptr(),
            0,
            &mut info,
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        ) == 0
        {
            return None;
        }

        let screen = GetDC(std::ptr::null_mut());
        let dc = CreateCompatibleDC(screen);
        let mut header: BITMAPINFO = std::mem::zeroed();
        header.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        header.bmiHeader.biWidth = pixels as i32;
        header.bmiHeader.biHeight = -(pixels as i32);
        header.bmiHeader.biPlanes = 1;
        header.bmiHeader.biBitCount = 32;
        header.bmiHeader.biCompression = BI_RGB;
        let mut data = std::ptr::null_mut();
        let bitmap = CreateDIBSection(
            dc,
            &header,
            DIB_RGB_COLORS,
            &mut data,
            std::ptr::null_mut(),
            0,
        );
        let result = if !dc.is_null() && !bitmap.is_null() && !data.is_null() {
            let previous = SelectObject(dc, bitmap);
            std::ptr::write_bytes(data.cast::<u8>(), 0, pixels as usize * pixels as usize * 4);
            let drawn = DrawIconEx(
                dc,
                0,
                0,
                info.hIcon,
                pixels as i32,
                pixels as i32,
                0,
                std::ptr::null_mut(),
                DI_NORMAL,
            );
            GdiFlush();
            let bytes = if drawn != 0 {
                Some(
                    std::slice::from_raw_parts(
                        data.cast::<u8>(),
                        pixels as usize * pixels as usize * 4,
                    )
                    .to_vec(),
                )
            } else {
                None
            };
            SelectObject(dc, previous);

            bytes
        } else {
            None
        };

        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        if !dc.is_null() {
            DeleteDC(dc);
        }
        if !screen.is_null() {
            ReleaseDC(std::ptr::null_mut(), screen);
        }
        DestroyIcon(info.hIcon);

        result.map(|bytes| (identity, bytes))
    }
}
