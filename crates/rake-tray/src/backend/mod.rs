//! Native adapters contain all host-specific resources and thread requirements.

use crate::{
    Backend, Capabilities, Error, Feature, Image, Rendering, Request, Support, Tray, host::Sink,
};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod macos_menu;
#[cfg(target_os = "macos")]
pub(crate) use macos::Item;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
mod windows_menu;
#[cfg(target_os = "windows")]
pub(crate) use windows::Item;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::Item;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
mod unsupported;
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
pub(crate) use unsupported::Item;

pub(crate) fn check_thread() -> Result<(), Error> {
    Item::check_thread()
}
pub(crate) fn create(application: &str, spec: &Tray, sink: Sink) -> Request<Item> {
    Item::create(application, spec, sink)
}

pub(crate) fn capabilities() -> Capabilities {
    let backend = if cfg!(target_os = "macos") {
        Backend::AppKit
    } else if cfg!(target_os = "windows") {
        Backend::Windows
    } else if cfg!(target_os = "linux") {
        Backend::StatusNotifier
    } else {
        Backend::Unsupported
    };
    let mut features = std::collections::BTreeMap::new();

    if backend != Backend::Unsupported {
        for feature in [
            Feature::MultipleItems,
            Feature::Image,
            Feature::Tooltip,
            Feature::PersistentIdentity,
            Feature::Menu,
            Feature::MenuChecks,
        ] {
            features.insert(feature, Support::Supported);
        }
    }
    if backend == Backend::AppKit {
        for feature in [
            Feature::TemplateImage,
            Feature::NamedImage,
            Feature::Rectangle,
            Feature::PositionHint,
            Feature::ShowMenu,
            Feature::MenuMixed,
            Feature::MenuImages,
            Feature::InlineText,
            Feature::ItemLength,
            Feature::UserRemoval,
            Feature::DisabledAppearance,
            Feature::DisableInteraction,
            Feature::MenuAlternates,
            Feature::MenuAccelerators,
        ] {
            features.insert(feature, Support::Supported);
        }
    }
    if backend == Backend::AppKit {
        for feature in [
            Feature::MenuSubtitles,
            Feature::MenuBadges,
            Feature::MenuPalette,
        ] {
            features.insert(
                feature,
                Support::Conditional(
                    "requires the corresponding AppKit runtime selector (macOS 14/14.4+)",
                ),
            );
        }
    }
    if backend == Backend::Windows {
        features.insert(Feature::ShowMenu, Support::Supported);
        features.insert(Feature::MenuImages, Support::Supported);
        features.insert(
            Feature::Rectangle,
            Support::Conditional("icon can be hidden or in overflow"),
        );
    }
    if backend == Backend::StatusNotifier {
        features.insert(
            Feature::MenuAccelerators,
            Support::Conditional("DBusMenu shortcut execution is owned by the host"),
        );
        features.insert(Feature::MenuImages, Support::Supported);
        features.insert(Feature::NamedImage, Support::Supported);
        features.insert(
            Feature::PositionHint,
            Support::Conditional("host-supplied coordinates are hints"),
        );
        features.insert(Feature::Scroll, Support::Supported);
        for feature in [Feature::RichTooltip, Feature::Overlay, Feature::Attention] {
            features.insert(feature, Support::Supported);
        }
    }

    Capabilities { backend, features }
}

pub(crate) fn validate(spec: &Tray) -> Result<(), Error> {
    let capabilities = capabilities();

    if capabilities.backend() == Backend::Unsupported {
        return Err(Error::Unsupported(Feature::MultipleItems));
    }
    if spec.macos.is_some() && !cfg!(target_os = "macos") {
        return Err(Error::Unsupported(Feature::InlineText));
    }
    if spec.linux.is_some() && !cfg!(target_os = "linux") {
        return Err(Error::Unsupported(Feature::RichTooltip));
    }
    if let Some(mac) = &spec.macos
        && let Some(image) = &mac.alternate_image
    {
        validate_image(image)?;
    }
    if let Some(linux) = &spec.linux {
        for image in [
            &linux.overlay,
            &linux.attention,
            &linux.tooltip.as_ref().and_then(|tip| tip.image.clone()),
        ]
        .into_iter()
        .flatten()
        {
            validate_image(image)?;
        }
    }
    if spec.image.is_none() && !spec.macos.as_ref().is_some_and(|mac| !mac.title.is_empty()) {
        return Err(Error::InvalidInput("tray item requires an image".into()));
    }
    if let Some(menu) = &spec.menu {
        validate_menu(menu)?;
    }
    if let Some(image) = &spec.image {
        validate_image(image)?;
    }
    if spec.menu.is_some() && matches!(capabilities.support(Feature::Menu), Support::Unsupported(_))
    {
        return Err(Error::Unsupported(Feature::Menu));
    }
    if cfg!(target_os = "windows") && spec.tooltip.encode_utf16().count() > 127 {
        return Err(Error::InvalidInput(
            "Windows tooltip exceeds 127 UTF-16 code units".into(),
        ));
    }

    Ok(())
}

fn validate_image(image: &Image) -> Result<(), Error> {
    if image.rendering == Rendering::Template && !cfg!(target_os = "macos") {
        return Err(Error::Unsupported(Feature::TemplateImage));
    }
    if cfg!(target_os = "windows") && image.representations.is_empty() {
        return Err(Error::Unsupported(Feature::NamedImage));
    }

    Ok(())
}

fn validate_menu(menu: &crate::Menu) -> Result<(), Error> {
    if menu.macos.is_some() && !cfg!(target_os = "macos") {
        return Err(Error::Unsupported(Feature::MenuPalette));
    }
    for item in &menu.items {
        if item.windows.is_some() && !cfg!(target_os = "windows") {
            return Err(Error::Unsupported(Feature::Menu));
        }
        if item.macos.is_some() && !cfg!(target_os = "macos") {
            return Err(Error::Unsupported(Feature::MenuAlternates));
        }
        if item.accelerator.is_some() && cfg!(target_os = "windows") {
            return Err(Error::Unsupported(Feature::MenuAccelerators));
        }
        if matches!(item.kind, crate::ItemKind::Check(crate::CheckState::Mixed))
            && !cfg!(target_os = "macos")
        {
            return Err(Error::Unsupported(Feature::MenuMixed));
        }
        if let Some(image) = &item.image {
            validate_image(image)?;
        }
        if let crate::ItemKind::Submenu(menu) = &item.kind {
            validate_menu(menu)?;
        }
    }
    Ok(())
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn mnemonic(label: &str, key: Option<char>, marker: char) -> String {
    let mut result = String::new();
    let mut marked = false;
    for ch in label.chars() {
        if !marked && key.is_some_and(|key| key.to_lowercase().eq(ch.to_lowercase())) {
            result.push(marker);
            marked = true;
        }
        result.push(ch);
        if ch == marker {
            result.push(marker);
        }
    }
    result
}
