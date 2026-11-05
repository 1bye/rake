//! Native-resource-free tray item specifications.

use crate::{Error, Image, Menu};

/// How an item behaves when a compatible panel is absent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HostPolicy {
    /// Fail creation when a native host cannot accept the item.
    Require,
    /// Export the item and observe a panel appearing later.
    #[default]
    Wait,
}

/// Primary interaction has exactly one owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interaction {
    /// Follow native menu behavior when a menu is attached.
    #[default]
    NativeMenu,
    /// Emit primary activation to application code.
    Events,
    /// Request an application-owned popup.
    Popup,
}

/// Description of one independent tray item; native work starts at registration.
#[derive(Clone, Debug)]
pub struct Tray {
    pub(crate) id: String,
    pub(crate) image: Option<Image>,
    pub(crate) tooltip: String,
    pub(crate) menu: Option<Menu>,
    pub(crate) visible: bool,
    pub(crate) interaction: Interaction,
    pub(crate) host_policy: HostPolicy,
}

impl Tray {
    /// Describes a visible item with the supplied stable application-scoped name.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            image: None,
            tooltip: String::new(),
            menu: None,
            visible: true,
            interaction: Interaction::NativeMenu,
            host_policy: HostPolicy::Wait,
        }
    }
    /// Sets the image source.
    pub fn icon(mut self, image: Image) -> Self {
        self.image = Some(image);
        self
    }
    /// Sets plain native tooltip text.
    pub fn tooltip(mut self, text: impl Into<String>) -> Self {
        self.tooltip = text.into();
        self
    }
    /// Attaches a separately projected menu description.
    pub fn menu(mut self, menu: Menu) -> Self {
        self.menu = Some(menu);
        self
    }
    /// Selects the owner of primary activation.
    pub fn primary(mut self, interaction: Interaction) -> Self {
        self.interaction = interaction;
        self
    }
    /// Chooses whether creation can wait for a native panel.
    pub fn host_policy(mut self, policy: HostPolicy) -> Self {
        self.host_policy = policy;
        self
    }
    /// Describes an initially hidden item.
    pub fn hidden(mut self) -> Self {
        self.visible = false;
        self
    }
    /// Returns the application-stable name.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the described native menu.
    pub fn menu_description(&self) -> Option<&Menu> {
        self.menu.as_ref()
    }
    /// Returns the selected primary interaction policy.
    pub fn primary_interaction(&self) -> Interaction {
        self.interaction
    }

    /// Validates identity, text and menu before native resource allocation.
    /// # Errors
    /// Rejects invalid names, NUL/excessive tooltip text and malformed menus.
    pub fn validate(&self) -> Result<(), Error> {
        crate::identifier(&self.id)?;
        crate::text(&self.tooltip, "tooltip", 16 * 1024)?;

        if let Some(menu) = &self.menu {
            menu.validate()?;
        }

        Ok(())
    }
}
