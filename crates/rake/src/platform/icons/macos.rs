//! NSWorkspace lookups and exact-size bitmap drawing, called only by blocking workers.

use super::{
    Identity,
    cache::{Cache, DEFAULT_BUDGET, Key},
};
use objc2::AnyThread;
use objc2_app_kit::{
    NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace, NSGraphicsContext, NSWorkspace,
};
use objc2_foundation::{NSBundle, NSPoint, NSRect, NSSize, NSString, NSURL, NSURLContentTypeKey};
use std::{
    path::Path,
    sync::{Mutex, OnceLock},
};

pub(super) fn load(path: &Path, pixels: u32) -> Option<(Identity, Vec<u8>)> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| Mutex::new(Cache::new(DEFAULT_BUDGET)))
        .lock()
        .ok()?;

    load_cached(path, pixels, &mut cache)
}

fn load_cached(path: &Path, pixels: u32, cache: &mut Cache) -> Option<(Identity, Vec<u8>)> {
    objc2::rc::autoreleasepool(|_| {
        let identity = identity(path);
        let key = Key {
            identity: identity.clone(),
            pixels,
        };

        if let Some(bytes) = cache.get(&key) {
            return Some((identity, bytes.to_vec()));
        }

        let path = NSString::from_str(path.to_str()?);
        let icon = NSWorkspace::sharedWorkspace().iconForFile(&path);
        // SAFETY: Null planes asks AppKit to own a packed four-channel 8-bit bitmap.
        // Dimensions and row stride are bounded by the caller to at most 256 pixels.
        let bitmap = unsafe {
            NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                NSBitmapImageRep::alloc(), std::ptr::null_mut(), pixels as isize,
                pixels as isize, 8, 4, true, false, NSDeviceRGBColorSpace,
                pixels as isize * 4, 32,
            )
        }?;
        let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&bitmap)?;
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&context));
        let rect = NSRect::new(
            NSPoint::new(0., 0.),
            NSSize::new(pixels as f64, pixels as f64),
        );
        icon.drawInRect_fromRect_operation_fraction(
            rect,
            NSRect::ZERO,
            NSCompositingOperation::Copy,
            1.,
        );
        NSGraphicsContext::restoreGraphicsState_class();
        let data = bitmap.bitmapData();

        if data.is_null() {
            return None;
        }

        // SAFETY: The bitmap owns pixels * pixels packed RGBA pixels until this scope ends.
        let mut bytes =
            unsafe { std::slice::from_raw_parts(data, pixels as usize * pixels as usize * 4) }
                .to_vec();

        // GPUI's RenderImage expects BGRA channels.
        for pixel in bytes.as_chunks_mut::<4>().0 {
            pixel.swap(0, 2);
        }

        cache.insert(key, bytes.clone().into());

        Some((identity, bytes))
    })
}

fn identity(path: &Path) -> Identity {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let custom = CString::new(path.as_os_str().as_bytes())
        .ok()
        .is_some_and(|path| {
            let mut info = [0u8; 32];
            // SAFETY: Both C strings are terminated and the output buffer has the supplied length.
            let count = unsafe {
                libc::getxattr(
                    path.as_ptr(),
                    c"com.apple.FinderInfo".as_ptr(),
                    info.as_mut_ptr().cast(),
                    info.len(),
                    0,
                    0,
                )
            };

            count >= 10 && u16::from_be_bytes([info[8], info[9]]) & 0x0400 != 0
        });

    if custom || path.is_symlink() {
        return Identity::Custom(path.to_path_buf());
    }

    if path.extension().is_some_and(|extension| extension == "app") {
        if let Some(identifier) = path
            .to_str()
            .and_then(|path| NSBundle::bundleWithPath(&NSString::from_str(path)))
            .and_then(|bundle| bundle.bundleIdentifier())
        {
            return Identity::Bundle(identifier.to_string());
        }

        return Identity::Custom(path.to_path_buf());
    }

    if path.is_dir() {
        return Identity::Folder;
    }

    if let Some(path) = path.to_str() {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path));
        let mut value = None;
        // SAFETY: The resource key is the Foundation type-identifier string key.
        if unsafe { url.getResourceValue_forKey_error(&mut value, NSURLContentTypeKey) }.is_ok()
            && let Some(value) = value
        {
            // SAFETY: NSURLContentTypeKey returns UTType, whose identifier is an NSString.
            let identifier: objc2::rc::Retained<NSString> =
                unsafe { objc2::msg_send![&*value, identifier] };

            return Identity::FileType(identifier.to_string());
        }
    }

    Identity::FileType(
        path.extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_thousand_real_folder_paths_share_one_native_raster() {
        let root = tempfile::tempdir().unwrap();
        let mut cache = Cache::new(DEFAULT_BUDGET);

        for index in 0..10_000 {
            let path = root.path().join(format!("folder-{index}"));
            std::fs::create_dir(&path).unwrap();
            let (identity, bytes) = load_cached(&path, 44, &mut cache).unwrap();
            assert_eq!(identity, Identity::Folder);
            assert_eq!(bytes.len(), 44 * 44 * 4);
        }

        assert_eq!(cache.stats(), (1, 44 * 44 * 4, 9_999, 1));
    }
}
