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
