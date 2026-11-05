//! Operation-specific support, independent of shell visibility.

/// Native implementation selected by the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Backend {
    /// Apple AppKit menu-bar items.
    AppKit,
    /// Windows Shell notification-area icons.
    Windows,
    /// Linux StatusNotifierItem with DBusMenu.
    StatusNotifier,
    /// Legacy X11 embedded icons.
    XEmbed,
    /// No native desktop implementation.
    Unsupported,
}

/// One independently queried tray operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Feature {
    /// Create multiple independent items.
    MultipleItems,
    /// Replace a raster icon.
    Image,
    /// Native monochrome template rendering.
    TemplateImage,
    /// Native named/theme image lookup.
    NamedImage,
    /// Plain tooltip text.
    Tooltip,
    /// Tooltip icon, title and body.
    RichTooltip,
    /// Visible text beside an icon.
    InlineText,
    /// Native overlay image.
    Overlay,
    /// Native attention status/images.
    Attention,
    /// Native menus and submenus.
    Menu,
    /// Check, mixed and radio menu items.
    MenuChecks,
    /// Menu item images.
    MenuImages,
    /// Native menu accelerator execution.
    MenuAccelerators,
    /// Menu item subtitles.
    MenuSubtitles,
    /// Native menu item badges.
    MenuBadges,
    /// Alternate native menu items.
    MenuAlternates,
    /// Native palette menu presentation.
    MenuPalette,
    /// Custom native menu views/drawing.
    MenuCustomContent,
    /// Actual pointer event detail.
    PointerEvents,
    /// Native double-click reporting.
    DoubleClick,
    /// Pointer enter/leave observation.
    Hover,
    /// Scroll requests.
    Scroll,
    /// Tray icon as a drop destination.
    DropTarget,
    /// Current exact icon rectangle.
    Rectangle,
    /// At least a host-supplied coordinate hint.
    PositionHint,
    /// Arbitrary programmatic native menu opening.
    ShowMenu,
    /// Native user-removal policy and observation.
    UserRemoval,
    /// Native persistence key.
    PersistentIdentity,
    /// Native item length.
    ItemLength,
    /// Appearance-only disabled state.
    DisabledAppearance,
    /// True interaction disabling.
    DisableInteraction,
    /// Native legacy balloon notification.
    LegacyBalloon,
    /// An application-owned rich popup window.
    Popup,
    /// OS-controlled expanded-interface sessions.
    ExpandedInterface,
    /// Native label sizing/ordering hints.
    OrderingHint,
    /// Frame animation rendered by the application.
    Animation,
}

/// Knowledge about a native feature, not a promise of successful presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// Implemented by the adapter.
    Supported,
    /// Not implemented or unavailable; includes an actionable reason.
    Unsupported(&'static str),
    /// Depends on the current shell/runtime and cannot be guaranteed in advance.
    Conditional(&'static str),
}

/// Backend capability snapshot.
#[derive(Clone, Debug)]
pub struct Capabilities {
    pub(crate) backend: Backend,
    pub(crate) features: std::collections::BTreeMap<Feature, Support>,
}

impl Capabilities {
    /// Returns the selected native implementation.
    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// Queries a feature without triggering native work.
    pub fn support(&self, feature: Feature) -> Support {
        self.features
            .get(&feature)
            .copied()
            .unwrap_or(Support::Unsupported("not implemented by this adapter"))
    }
}
