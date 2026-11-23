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

    let configured = configured_theme();
    let themes = [configured.as_deref().unwrap_or(""), "Adwaita", "hicolor"];
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

fn configured_theme() -> Option<String> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;

    for (file, section, key) in [
        ("gtk-3.0/settings.ini", "Settings", "gtk-icon-theme-name"),
        ("gtk-4.0/settings.ini", "Settings", "gtk-icon-theme-name"),
        ("kdeglobals", "Icons", "Theme"),
    ] {
        let Ok(text) = std::fs::read_to_string(config.join(file)) else {
            continue;
        };
        if let Some(theme) = setting(&text, section, key) {
            return Some(theme.to_owned());
        }
    }

    let output = Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .env("LC_ALL", "C")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim().trim_matches('\'');

    (!value.is_empty()).then(|| value.to_owned())
}

fn setting<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut current = "";

    for line in text.lines().map(str::trim) {
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            current = name;
        } else if current == section
            && let Some((name, value)) = line.split_once('=')
            && name.trim() == key
            && !value.trim().is_empty()
        {
            return Some(value.trim());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn icon_theme_setting_respects_section_and_ignores_empty_values() {
        let text = "[Other]\nTheme=wrong\n[Icons]\nTheme=\n Theme = chosen \n";

        assert_eq!(super::setting(text, "Icons", "Theme"), Some("chosen"));
        assert_eq!(
            super::setting(text, "Settings", "gtk-icon-theme-name"),
            None
        );
    }
}
