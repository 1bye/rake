//! Covers: portable tray validation and menu dispatch boundaries.
//! Real: public tray/image/menu contracts.
//! Doubles: none.
//! Requires: no desktop session.

use rake_tray::{CheckState, Image, ImageRepresentation, ItemKind, Menu, MenuItem, Tray};

#[test]
fn malformed_images_and_untrusted_strings_fail_before_registration() {
    assert!(ImageRepresentation::rgba(u32::MAX, 2, vec![]).is_err());
    assert!(ImageRepresentation::rgba(2, 2, vec![0; 15]).is_err());
    assert!(Image::named("bad\0name").is_err());
    assert!(Tray::new("../escape").validate().is_err());
    assert!(Tray::new("main").tooltip("bad\0tip").validate().is_err());
}

#[test]
fn menu_validation_checks_nested_ids_and_radio_exclusivity() {
    let nested = Menu::new().item(MenuItem::new("open", "Nested"));
    let duplicate = Menu::new()
        .item(MenuItem::new("open", "Open"))
        .item(MenuItem::new("sub", "Sub").kind(ItemKind::Submenu(nested)));
    assert!(duplicate.validate().is_err());

    let radio = |id| {
        MenuItem::new(id, id).kind(ItemKind::Radio {
            group: "profile".into(),
            selected: true,
        })
    };
    assert!(
        Menu::new()
            .item(radio("one"))
            .item(radio("two"))
            .validate()
            .is_err()
    );
}

#[test]
fn disabled_or_hidden_ancestors_prevent_stale_menu_dispatch() {
    let mut parent =
        MenuItem::new("parent", "Parent")
            .kind(ItemKind::Submenu(Menu::new().item(
                MenuItem::new("check", "Check").kind(ItemKind::Check(CheckState::Mixed)),
            )));
    let menu = Menu::new().item(parent.clone());
    assert!(menu.actionable("check"));
    parent.enabled = false;
    assert!(!Menu::new().item(parent.clone()).actionable("check"));
    parent.enabled = true;
    parent.visible = false;
    assert!(!Menu::new().item(parent).actionable("check"));
}

#[test]
fn native_text_and_accelerators_are_validated_without_a_desktop() {
    use rake_tray::{Accelerator, macos};
    let key = Accelerator::parse("Cmd+Shift+k").unwrap();
    assert!(key.super_key && key.shift);
    assert_eq!(key.key, "k");
    assert!(Accelerator::parse("Ctrl+Ctrl+k").is_err());
    assert!(Accelerator::parse("Meta+long").is_err());
    assert!(
        Tray::new("bad-width")
            .macos(macos::Appearance {
                length: macos::ItemLength::Points(f64::NAN),
                ..Default::default()
            })
            .validate()
            .is_err()
    );
    let mut item = MenuItem::new("mnemonic", "Open");
    item.access_key = Some('x');
    assert!(Menu::new().item(item).validate().is_err());
}

#[test]
fn composed_badges_preserve_straight_alpha_and_progress_bounds() {
    use rake_tray::{Animation, Corner};
    let transparent = Image::new(ImageRepresentation::rgba(4, 4, vec![0; 64]).unwrap());
    let badge = Image::new(ImageRepresentation::rgba(1, 1, vec![255, 0, 0, 128]).unwrap());
    let composed = transparent.badge(&badge, Corner::BottomRight, 0.5).unwrap();
    let bytes = composed.representations()[0].bytes();
    assert_eq!(
        &bytes[60..64],
        &[255, 0, 0, 128],
        "transparent pixels must not darken badge edges"
    );
    assert_eq!(&bytes[..4], &[0, 0, 0, 0]);
    assert_eq!(
        transparent
            .progress(0.0, 0.25, [0, 255, 0, 255])
            .unwrap()
            .representations()[0]
            .bytes(),
        transparent.representations()[0].bytes()
    );
    assert!(transparent.progress(f32::NAN, 0.1, [0; 4]).is_err());
    assert!(transparent.badge(&badge, Corner::TopLeft, 1.1).is_err());
    assert!(
        Animation::new(
            vec![transparent.clone(), transparent],
            std::time::Duration::from_millis(1)
        )
        .is_err()
    );
    assert!(
        rake_tray::windows::LegacyBalloon::new("x", "💡".repeat(128))
            .validate()
            .is_err()
    );
}
