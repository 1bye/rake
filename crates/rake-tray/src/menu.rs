//! Stable-keyed native menu descriptions with bounded tree validation.

use crate::{Error, Image};
use std::collections::{BTreeMap, BTreeSet};

/// Three-state native checkmark.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CheckState {
    /// Unselected.
    #[default]
    Off,
    /// Selected.
    On,
    /// Indeterminate.
    Mixed,
}

/// Semantic menu item kind.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ItemKind {
    /// Guarded application quit, interpreted by the Rake host.
    Quit,
    /// Ordinary command item.
    Action,
    /// Independent check item.
    Check(CheckState),
    /// One selection in a named radio group.
    Radio {
        /// Stable group identifier.
        group: String,
        /// Whether this item is selected.
        selected: bool,
    },
    /// Non-interactive separator.
    Separator,
    /// Nested menu.
    Submenu(Menu),
    /// Non-interactive section heading.
    Header,
}

/// One native menu item; its ID survives label/state changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// Stable key within the complete attached menu tree.
    pub id: String,
    /// Plain text, never implicit mnemonic markup.
    pub label: String,
    /// Semantic item behavior.
    pub kind: ItemKind,
    /// Whether user activation is allowed.
    pub enabled: bool,
    /// Whether this item is projected into native menus.
    pub visible: bool,
    /// Optional item image.
    pub image: Option<Image>,
    /// Optional menu-scoped accelerator, never a global registration.
    pub accelerator: Option<String>,
}

impl MenuItem {
    /// Describes an enabled command item.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind: ItemKind::Action,
            enabled: true,
            visible: true,
            image: None,
            accelerator: None,
        }
    }

    /// Describes a separator with an explicit stable ID.
    pub fn separator(id: impl Into<String>) -> Self {
        Self {
            kind: ItemKind::Separator,
            enabled: false,
            ..Self::new(id, "")
        }
    }

    /// Describes a guarded application quit item.
    pub fn quit(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(id, label).kind(ItemKind::Quit)
    }

    /// Selects the semantic kind.
    pub fn kind(mut self, kind: ItemKind) -> Self {
        self.kind = kind;
        self
    }
}

/// An application-owned menu description, independently projected per tray.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Menu {
    /// Root menu items in display order.
    pub items: Vec<MenuItem>,
}

impl Menu {
    /// Creates an empty menu.
    pub fn new() -> Self {
        Self::default()
    }
    /// Appends a described item.
    pub fn item(mut self, item: MenuItem) -> Self {
        self.items.push(item);
        self
    }

    /// Checks all keys, strings, group selections and tree limits before native work.
    /// # Errors
    /// Rejects duplicate/invalid IDs, excessive depth/nodes or multiple selected radios.
    pub fn validate(&self) -> Result<(), Error> {
        let mut keys = BTreeSet::new();
        let mut radios = BTreeMap::new();

        self.validate_at(0, &mut keys, &mut radios)
    }

    fn validate_at(
        &self,
        depth: usize,
        keys: &mut BTreeSet<String>,
        radios: &mut BTreeMap<String, usize>,
    ) -> Result<(), Error> {
        if depth > 32 {
            return Err(Error::InvalidInput("menu exceeds depth 32".into()));
        }

        for item in &self.items {
            crate::identifier(&item.id)?;
            crate::text(&item.label, "menu label", 4096)?;

            if !keys.insert(item.id.clone()) {
                return Err(Error::InvalidInput(format!(
                    "duplicate menu ID: {}",
                    item.id
                )));
            }
            if keys.len() > 1024 {
                return Err(Error::InvalidInput("menu exceeds 1024 items".into()));
            }
            if let Some(accelerator) = &item.accelerator {
                crate::text(accelerator, "accelerator", 128)?;
            }

            match &item.kind {
                ItemKind::Submenu(menu) => menu.validate_at(depth + 1, keys, radios)?,
                ItemKind::Radio { group, selected } => {
                    crate::identifier(group)?;
                    let count = radios.entry(group.clone()).or_default();
                    *count += usize::from(*selected);

                    if *count > 1 {
                        return Err(Error::InvalidInput(format!(
                            "radio group {group} has multiple selections"
                        )));
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Finds an item by its stable key, including nested menus.
    pub fn find(&self, id: &str) -> Option<&MenuItem> {
        self.items.iter().find_map(|item| {
            if item.id == id {
                return Some(item);
            }
            if let ItemKind::Submenu(menu) = &item.kind {
                return menu.find(id);
            }
            None
        })
    }

    /// Resolves an actionable, currently enabled/visible item across nested menus.
    pub fn actionable(&self, id: &str) -> bool {
        self.items.iter().any(|item| {
            item.visible
                && item.enabled
                && match &item.kind {
                    ItemKind::Submenu(menu) => menu.actionable(id),
                    ItemKind::Action
                    | ItemKind::Quit
                    | ItemKind::Check(_)
                    | ItemKind::Radio { .. } => item.id == id,
                    _ => false,
                }
        })
    }
}
