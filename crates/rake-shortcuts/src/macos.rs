//! Carbon event hotkeys need no Accessibility or Input Monitoring permission.
use super::Error;
use futures::channel::mpsc;
use rake_tray::Accelerator;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

#[repr(C)]
#[derive(Clone, Copy)]
struct HotKeyId {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventType {
    class: u32,
    kind: u32,
}

type Ref = *mut c_void;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardLayoutInputSource() -> Ref;
    fn TISGetInputSourceProperty(source: Ref, property: Ref) -> Ref;
    static kTISPropertyUnicodeKeyLayoutData: Ref;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const c_void,
        key: u16,
        action: u16,
        modifiers: u32,
        keyboard: u32,
        options: u32,
        dead: *mut u32,
        capacity: u32,
        length: *mut u32,
        output: *mut u16,
    ) -> i32;
    fn GetApplicationEventTarget() -> Ref;
    fn InstallEventHandler(
        target: Ref,
        callback: unsafe extern "C" fn(Ref, Ref, Ref) -> i32,
        count: u32,
        types: *const EventType,
        data: Ref,
        handler: *mut Ref,
    ) -> i32;
    fn RemoveEventHandler(handler: Ref) -> i32;
    fn RegisterEventHotKey(
        key: u32,
        modifiers: u32,
        id: HotKeyId,
        target: Ref,
        options: u32,
        hotkey: *mut Ref,
    ) -> i32;
    fn UnregisterEventHotKey(hotkey: Ref) -> i32;
    fn GetEventParameter(
        event: Ref,
        name: u32,
        kind: u32,
        actual_kind: *mut u32,
        size: u32,
        actual_size: *mut u32,
        data: Ref,
    ) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: Ref) -> *const u8;
    fn CFRelease(value: Ref);
}

struct Delivery {
    id: HotKeyId,
    sender: mpsc::Sender<()>,
}

pub(super) struct Registration {
    hotkey: Ref,
    handler: Ref,
    _delivery: Box<Delivery>,
}

unsafe extern "C" fn activated(_: Ref, event: Ref, data: Ref) -> i32 {
    let mut id = HotKeyId {
        signature: 0,
        id: 0,
    };
    // SAFETY: Carbon supplies a live event and the handler's boxed Delivery. Both
    // remain valid until RemoveEventHandler on this same main thread.
    unsafe {
        if GetEventParameter(
            event,
            u32::from_be_bytes(*b"----"),
            u32::from_be_bytes(*b"hkid"),
            std::ptr::null_mut(),
            std::mem::size_of::<HotKeyId>() as u32,
            std::ptr::null_mut(),
            (&mut id as *mut HotKeyId).cast(),
        ) != 0
        {
            return -9874;
        }

        let delivery = &mut *data.cast::<Delivery>();

        if id.signature != delivery.id.signature || id.id != delivery.id.id {
            return -9874;
        }

        let _ = delivery.sender.try_send(());
    }

    0
}

pub(super) async fn register(
    accelerator: Accelerator,
    sender: mpsc::Sender<()>,
) -> Result<Registration, Error> {
    if objc2::MainThreadMarker::new().is_none() {
        return Err(Error::Native("register on the main UI thread".into()));
    }

    let (key, implicit_shift) = keycode(&accelerator.key)?;
    let modifiers = u32::from(accelerator.super_key) * (1 << 8)
        | u32::from(accelerator.shift || implicit_shift) * (1 << 9)
        | u32::from(accelerator.alt) * (1 << 11)
        | u32::from(accelerator.control) * (1 << 12);
    static NEXT: AtomicU32 = AtomicU32::new(1);
    let id = NEXT
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .map_err(|_| Error::Native("shortcut IDs exhausted".into()))?;
    let mut delivery = Box::new(Delivery {
        id: HotKeyId {
            signature: u32::from_be_bytes(*b"RAKE"),
            id,
        },
        sender,
    });
    let data = (&mut *delivery as *mut Delivery).cast();
    let mut registration = Registration {
        hotkey: std::ptr::null_mut(),
        handler: std::ptr::null_mut(),
        _delivery: delivery,
    };
    let event_type = EventType {
        class: u32::from_be_bytes(*b"keyb"),
        kind: 6,
    };
    // SAFETY: All pointers reference initialized storage. The boxed delivery has
    // a stable address and Registration removes the handler before freeing it.
    unsafe {
        let target = GetApplicationEventTarget();
        status(InstallEventHandler(
            target,
            activated,
            1,
            &event_type,
            data,
            &mut registration.handler,
        ))?;
        let result = RegisterEventHotKey(
            key,
            modifiers,
            registration._delivery.id,
            target,
            0,
            &mut registration.hotkey,
        );

        if result == -9878 {
            return Err(Error::Conflict);
        }

        status(result)?;
    }

    Ok(registration)
}

impl Registration {
    pub(super) async fn unregister(&mut self) -> Result<(), Error> {
        self.release()
    }

    fn release(&mut self) -> Result<(), Error> {
        // SAFETY: Resources are owned by this registration on the main thread.
        unsafe {
            if !self.hotkey.is_null() {
                status(UnregisterEventHotKey(self.hotkey))?;
                self.hotkey = std::ptr::null_mut();
            }

            if !self.handler.is_null() {
                status(RemoveEventHandler(self.handler))?;
                self.handler = std::ptr::null_mut();
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

fn status(code: i32) -> Result<(), Error> {
    if code == 0 {
        Ok(())
    } else {
        Err(Error::Native(format!("Carbon status {code}")))
    }
}

fn keycode(key: &str) -> Result<(u32, bool), Error> {
    // SAFETY: The copied input source owns its layout data until CFRelease. The
    // translator receives initialized output/dead-key storage with matching sizes.
    unsafe {
        let source = TISCopyCurrentKeyboardLayoutInputSource();

        if source.is_null() {
            return Err(Error::Native(
                "current keyboard layout is unavailable".into(),
            ));
        }

        let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
        let mut found = None;

        if !data.is_null() {
            let layout = CFDataGetBytePtr(data).cast();
            let expected: Vec<u16> = key.encode_utf16().collect();

            for shift in [false, true] {
                for code in 0..128 {
                    let mut output = [0_u16; 4];
                    let mut length = 0;
                    let mut dead = 0;
                    let status = UCKeyTranslate(
                        layout,
                        code,
                        0,
                        if shift { 2 } else { 0 },
                        u32::from(LMGetKbdType()),
                        1,
                        &mut dead,
                        output.len() as u32,
                        &mut length,
                        output.as_mut_ptr(),
                    );

                    if status == 0 && output.get(..length as usize) == Some(expected.as_slice()) {
                        found = Some((u32::from(code), shift));
                        break;
                    }
                }

                if found.is_some() {
                    break;
                }
            }
        }

        CFRelease(source);

        found.ok_or_else(|| {
            Error::InvalidInput("key is absent from the current macOS keyboard layout".into())
        })
    }
}
