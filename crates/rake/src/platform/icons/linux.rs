//! Best-effort freedesktop MIME/theme lookup on blocking workers.

use super::Identity;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub(super) fn load(path: &Path, pixels: u32) -> Option<(Identity, Vec<u8>)> {
    let folder = path.is_dir();
    let mime = if folder {
        "inode/directory".to_owned()
    } else {
        let output = Command::new("xdg-mime")
            .args(["query", "filetype"])
            .arg(path)
            .env("LC_ALL", "C")
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        String::from_utf8(output.stdout).ok()?.trim().to_owned()
    };
    let identity = if folder {
        Identity::Folder
    } else {
        Identity::FileType(mime.clone())
    };
    let names = if folder {
        vec!["folder".to_owned()]
    } else {
        vec![
            mime.replace('/', "-"),
            format!("{}-x-generic", mime.split('/').next()?),
        ]
    };
    let mut roots = Vec::new();

    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(&home).join(".icons"));
        roots.push(
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(home).join(".local/share"))
                .join("icons"),
        );
    }

    for root in std::env::var_os("XDG_DATA_DIRS")
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into())
        .to_string_lossy()
        .split(':')
    {
        roots.push(PathBuf::from(root).join("icons"));
    }

    let configured = std::env::var("GTK_THEME").unwrap_or_default();
    let themes = [
        configured.split(':').next().unwrap_or(""),
        "Adwaita",
        "hicolor",
    ];
    let mut sizes = [16u32, 22, 24, 32, 48, 64, 96, 128, 256, 512];
    sizes.sort_by_key(|size| size.abs_diff(pixels));

    for theme in themes.into_iter().filter(|theme| !theme.is_empty()) {
        for root in &roots {
            for size in sizes {
                for category in ["mimetypes", "places"] {
                    for name in &names {
                        let file = root
                            .join(theme)
                            .join(format!("{size}x{size}"))
                            .join(category)
                            .join(format!("{name}.png"));
                        let Ok(mut reader) = image::ImageReader::open(file) else {
                            continue;
                        };
                        let mut limits = image::Limits::default();
                        limits.max_image_width = Some(1024);
                        limits.max_image_height = Some(1024);
                        limits.max_alloc = Some(4 * 1024 * 1024);
                        reader.limits(limits);
                        let Ok(image) = reader.decode() else {
                            continue;
                        };
                        let mut bytes = image
                            .resize_exact(pixels, pixels, image::imageops::FilterType::Lanczos3)
                            .into_rgba8()
                            .into_raw();

                        for pixel in bytes.as_chunks_mut::<4>().0 {
                            pixel.swap(0, 2);
                        }

                        return Some((identity, bytes));
                    }
                }
            }
        }
    }

    None
}
