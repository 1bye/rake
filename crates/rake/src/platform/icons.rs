//! Native file icons rasterized for the current display, with bounded shared storage.

use crate::gpui::{
    App, AppContext, Entity, Global, ImageCache, ImageCacheError, ImageSource, Pixels, RenderImage,
    Resource, Window,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

#[path = "icon_cache.rs"]
mod cache;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

use cache::{DEFAULT_BUDGET, Identity, Key};

/// A cheap-clone file icon. Cloning retains a path and shared cache, not image bytes.
#[derive(Clone)]
pub struct Icon {
    path: Arc<Path>,
    size: Pixels,
    cache: Entity<Icons>,
}

/// Returns an icon sized in logical pixels. No filesystem or native API runs here.
///
/// Rake initializes the shared cache before opening its windows. A generic file
/// placeholder appears while loading; give the `img` an explicit layout size.
/// Sizes are clamped to 1–256 physical pixels to bound individual allocations.
pub fn for_path(path: impl AsRef<Path>, size: Pixels, _window: &Window, cx: &App) -> Icon {
    Icon {
        path: path.as_ref().into(),
        size,
        cache: cx.global::<SharedIcons>().0.clone(),
    }
}

impl From<Icon> for ImageSource {
    fn from(icon: Icon) -> Self {
        ImageSource::from(move |window: &mut Window, cx: &mut App| {
            let pixels = physical_pixels(f32::from(icon.size), window.scale_factor());
            // The private cache resource appends size without converting native path bytes.
            let resource = Resource::Path(icon.path.join(pixels.to_string()).into());
            icon.cache
                .update(cx, |cache, cx| cache.load(&resource, window, cx))
        })
    }
}

/// Current cache counters for inspection and memory diagnostics.
#[derive(Clone, Copy, Debug)]
pub struct CacheStats {
    /// Number of distinct rendered identities and sizes.
    pub entries: usize,
    /// Retained rendered pixel bytes, excluding the 1 KiB placeholder and GPU storage.
    pub bytes: usize,
    /// Requests served from the rendered cache.
    pub hits: u64,
    /// Requests needing background classification or rasterization.
    pub misses: u64,
    /// Number of background loads currently in flight.
    pub pending: usize,
}

/// Reads current counters without filesystem or native calls.
pub fn inspect(cx: &App) -> CacheStats {
    let cache = cx.global::<SharedIcons>().0.read(cx);

    CacheStats {
        entries: cache.images.len(),
        bytes: cache.bytes,
        hits: cache.hits,
        misses: cache.misses,
        pending: cache.pending.len(),
    }
}

struct SharedIcons(Entity<Icons>);
impl Global for SharedIcons {}

pub(crate) fn init(cx: &mut App) {
    let icons = cx.new(|_| Icons::default());
    cx.set_global(SharedIcons(icons));
}

struct Raster {
    image: Arc<RenderImage>,
    used: u64,
    bytes: usize,
}

struct Icons {
    placeholder: Arc<RenderImage>,
    hits: u64,
    misses: u64,
    images: HashMap<Key, Raster>,
    paths: HashMap<PathBuf, Identity>,
    pending: std::collections::HashSet<PathBuf>,
    bytes: usize,
    clock: u64,
}

impl Default for Icons {
    fn default() -> Self {
        let buffer = image::RgbaImage::from_raw(16, 16, fallback(16, false))
            .expect("placeholder dimensions match its bitmap");

        Self {
            placeholder: Arc::new(RenderImage::new([image::Frame::new(buffer)])),
            hits: 0,
            misses: 0,
            images: HashMap::new(),
            paths: HashMap::new(),
            pending: std::collections::HashSet::new(),
            bytes: 0,
            clock: 0,
        }
    }
}

impl ImageCache for Icons {
    fn load(
        &mut self,
        resource: &Resource,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<Result<Arc<RenderImage>, ImageCacheError>> {
        let Resource::Path(request) = resource else {
            return None;
        };
        let pixels: u32 = request.file_name()?.to_str()?.parse().ok()?;
        let path = request.parent()?.to_path_buf();
        self.clock = self.clock.saturating_add(1);

        if let Some(identity) = self.paths.get(&path) {
            let key = Key {
                identity: identity.clone(),
                pixels,
            };
            if let Some(raster) = self.images.get_mut(&key) {
                raster.used = self.clock;
                self.hits += 1;

                return Some(Ok(raster.image.clone()));
            }
        }

        self.misses += 1;

        if self.pending.len() >= 16 || !self.pending.insert(request.to_path_buf()) {
            return Some(Ok(self.placeholder.clone()));
        }

        let entity = cx.global::<SharedIcons>().0.downgrade();
        let request = request.to_path_buf();
        cx.spawn(async move |cx| {
            #[cfg(not(target_family = "wasm"))]
            let loaded = blocking::unblock(move || load(&path, pixels)).await;
            #[cfg(target_family = "wasm")]
            let loaded = load(&path, pixels);

            let _ = entity.update(cx, |state, cx| {
                state.pending.remove(&request);
                let path = request.parent().unwrap_or(Path::new("")).to_path_buf();
                let (identity, rgba) = loaded;
                let key = Key {
                    identity: identity.clone(),
                    pixels,
                };

                if state.paths.len() >= 2048 {
                    state.paths.clear();
                }

                state.paths.insert(path, identity);

                if !state.images.contains_key(&key) {
                    let bytes = rgba.len();
                    while state.bytes + bytes > DEFAULT_BUDGET {
                        let oldest = state
                            .images
                            .iter()
                            .min_by_key(|(_, image)| image.used)
                            .map(|(key, _)| key.clone());
                        let Some(oldest) = oldest else {
                            break;
                        };

                        if let Some(old) = state.images.remove(&oldest) {
                            state.bytes -= old.bytes;
                            cx.drop_image(old.image, None);
                        }
                    }

                    if let Some(buffer) = image::RgbaImage::from_raw(pixels, pixels, rgba) {
                        let image = Arc::new(RenderImage::new([image::Frame::new(buffer)]));
                        state.bytes += bytes;
                        state.images.insert(
                            key,
                            Raster {
                                image,
                                bytes,
                                used: state.clock,
                            },
                        );
                    }
                }

                cx.refresh_windows();
            });
        })
        .detach();

        Some(Ok(self.placeholder.clone()))
    }
}

fn load(path: &Path, pixels: u32) -> (Identity, Vec<u8>) {
    #[cfg(target_os = "windows")]
    if let Some(raster) = windows::load(path, pixels) {
        return raster;
    }

    #[cfg(target_os = "linux")]
    if let Some(raster) = linux::load(path, pixels) {
        return raster;
    }

    #[cfg(target_os = "macos")]
    if let Some(raster) = macos::load(path, pixels) {
        return raster;
    }

    #[cfg(not(target_family = "wasm"))]
    let folder = path.is_dir();
    #[cfg(target_family = "wasm")]
    let folder = path.to_string_lossy().ends_with('/');
    let identity = if folder {
        Identity::Folder
    } else {
        Identity::FileType(
            path.extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase(),
        )
    };

    (identity, fallback(pixels, folder))
}

fn fallback(pixels: u32, folder: bool) -> Vec<u8> {
    let mut bytes = vec![0; pixels as usize * pixels as usize * 4];

    for y in pixels / 5..pixels * 4 / 5 {
        for x in pixels / 5..pixels * 4 / 5 {
            let offset = ((y * pixels + x) * 4) as usize;
            bytes[offset..offset + 4].copy_from_slice(if folder {
                &[70, 175, 225, 255]
            } else {
                &[180, 180, 180, 255]
            });
        }
    }

    bytes
}

fn physical_pixels(size: f32, scale: f32) -> u32 {
    let pixels = size * scale;

    if !pixels.is_finite() {
        return 1;
    }

    pixels.ceil().clamp(1., 256.) as u32
}

#[cfg(test)]
mod tests {
    #[test]
    fn display_scale_changes_the_raster_size() {
        assert_eq!(super::physical_pixels(22., 1.), 22);
        assert_eq!(super::physical_pixels(22., 2.), 44);
        assert_eq!(super::physical_pixels(22., 1.5), 33);
        assert_eq!(super::physical_pixels(f32::NAN, 2.), 1);
        assert_eq!(super::physical_pixels(-1., 2.), 1);
        assert_eq!(super::physical_pixels(1000., 2.), 256);
    }
}
