use crate::{CheckState, Error, EventKind, Input, ItemKind, Menu, host::Sink};
use std::{collections::BTreeMap, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::{DeleteObject, HBITMAP},
    UI::WindowsAndMessaging::*,
};

pub(super) struct NativeMenu {
    menu: HMENU,
    items: BTreeMap<u32, String>,
    sink: Sink,
    bitmaps: Vec<HBITMAP>,
}

impl NativeMenu {
    pub(super) fn new(spec: &Menu, sink: Sink) -> Result<Self, Error> {
        let mut items = BTreeMap::new();
        let mut bitmaps = Vec::new();
        let menu = match build(spec, &mut items, &mut bitmaps) {
            Ok(menu) => menu,
            Err(error) => {
                for bitmap in bitmaps {
                    /* SAFETY: Unattached, exclusively owned DIB. */
                    unsafe {
                        DeleteObject(bitmap);
                    }
                }
                return Err(error);
            }
        };
        Ok(Self {
            menu,
            items,
            sink,
            bitmaps,
        })
    }

    pub(super) fn show(&self, owner: HWND, x: i32, y: i32) -> Result<(), Error> {
        // SAFETY: Native menu and owner HWND are retained for the synchronous tracking lifetime.
        unsafe {
            SetForegroundWindow(owner);
            SetLastError(0);
            let command = TrackPopupMenuEx(
                self.menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON,
                x,
                y,
                owner,
                null_mut(),
            );
            let error = GetLastError();
            PostMessageW(owner, WM_NULL, 0, 0);
            if command == 0 && error != 0 {
                return Err(crate::native(
                    "show tray menu",
                    std::io::Error::from_raw_os_error(error as i32),
                ));
            }
            if let Some(id) = self.items.get(&(command as u32)) {
                self.sink
                    .send(EventKind::MenuSelected(id.clone()), None, Input::default());
            }
        }
        Ok(())
    }
}

impl Drop for NativeMenu {
    fn drop(&mut self) {
        // SAFETY: Root HMENU exclusively owns its native submenu tree.
        unsafe {
            DestroyMenu(self.menu);
            for bitmap in self.bitmaps.drain(..) {
                DeleteObject(bitmap);
            }
        }
    }
}

fn build(
    spec: &Menu,
    items: &mut BTreeMap<u32, String>,
    bitmaps: &mut Vec<HBITMAP>,
) -> Result<HMENU, Error> {
    // SAFETY: CreatePopupMenu has no caller preconditions.
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return Err(crate::native(
            "create tray menu",
            std::io::Error::last_os_error(),
        ));
    }
    let result = (|| {
        for item in &spec.items {
            if !item.visible {
                continue;
            }
            let id = items.len() as u32 + 1;
            items.insert(id, item.id.clone());
            let mut text: Vec<u16> = super::mnemonic(&item.label, item.access_key, '&')
                .encode_utf16()
                .chain(Some(0))
                .collect();
            let mut kind = match item.kind {
                ItemKind::Separator => MFT_SEPARATOR,
                ItemKind::Radio { .. } => MFT_RADIOCHECK,
                _ => MFT_STRING,
            };
            let appearance = item.windows.clone().unwrap_or_default();
            kind |= match appearance.column {
                crate::windows::Column::Continue => 0,
                crate::windows::Column::Break => MFT_MENUBREAK,
                crate::windows::Column::Divider => MFT_MENUBARBREAK,
            };
            if appearance.right_order {
                kind |= MFT_RIGHTORDER;
            }
            let bitmap = if let Some(image) = &item.image {
                let rep = image
                    .representations
                    .iter()
                    .min_by_key(|r| r.width.abs_diff(16))
                    .ok_or(Error::Unsupported(crate::Feature::NamedImage))?;
                let bitmap = super::windows::bitmap(rep)?;
                bitmaps.push(bitmap);
                bitmap
            } else {
                null_mut()
            };
            let submenu = if let ItemKind::Submenu(submenu) = &item.kind {
                build(submenu, items, bitmaps)?
            } else {
                null_mut()
            };
            let selected = matches!(
                item.kind,
                ItemKind::Check(CheckState::On) | ItemKind::Radio { selected: true, .. }
            );
            let info = MENUITEMINFOW {
                cbSize: size_of::<MENUITEMINFOW>() as u32,
                fMask: MIIM_FTYPE | MIIM_STATE | MIIM_STRING | MIIM_ID | MIIM_SUBMENU | MIIM_BITMAP,
                fType: kind,
                fState: (if selected { MFS_CHECKED } else { MFS_UNCHECKED })
                    | if appearance.default { MFS_DEFAULT } else { 0 }
                    | if item.enabled
                        && !matches!(item.kind, ItemKind::Header | ItemKind::Separator)
                    {
                        MFS_ENABLED
                    } else {
                        MFS_DISABLED
                    },
                wID: id,
                hSubMenu: submenu,
                hbmpItem: bitmap,
                dwTypeData: text.as_mut_ptr(),
                ..Default::default()
            };
            // SAFETY: Windows copies text/data and takes submenu ownership on successful insertion.
            if unsafe { InsertMenuItemW(menu, u32::MAX, 1, &info) } == 0 {
                if !submenu.is_null() {
                    // SAFETY: Failed insertion did not transfer submenu ownership.
                    unsafe {
                        DestroyMenu(submenu);
                    }
                }
                return Err(crate::native(
                    "insert tray menu item",
                    std::io::Error::last_os_error(),
                ));
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        // SAFETY: This root owns all successfully inserted submenus so far.
        unsafe {
            DestroyMenu(menu);
        }
        return Err(error);
    }
    Ok(menu)
}
