//! Native events retain optional input and coordinate evidence.

/// Physical or logical position as identified by its associated coordinate space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    /// Horizontal coordinate, possibly negative on another display.
    pub x: f64,
    /// Vertical coordinate.
    pub y: f64,
}

/// Native icon rectangle in a declared coordinate space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    /// Top-left or native-origin position, as specified by the coordinate space.
    pub origin: Point,
    /// Horizontal extent.
    pub width: f64,
    /// Vertical extent.
    pub height: f64,
}

/// The origin and units of geometry supplied by a backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoordinateSpace {
    /// Windows physical screen pixels, top-left origin.
    PhysicalScreen,
    /// AppKit logical screen points, bottom-left origin.
    AppKitScreen,
    /// Linux host hint whose global coordinate interpretation is host-defined.
    HostHint,
}

/// Placement evidence; absent geometry is never represented as a zero rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct Anchor {
    /// Exact icon rectangle when observable.
    pub rect: Option<Rect>,
    /// Pointer/host point hint when provided.
    pub point: Option<Point>,
    /// Native units/origin of the supplied geometry.
    pub space: CoordinateSpace,
}

/// Actual native pointer button when known.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// Primary mouse button.
    Primary,
    /// Secondary mouse button.
    Secondary,
    /// Middle button.
    Middle,
    /// Additional native button number.
    Other(u16),
}

/// Native input detail, without inventing pointer events for keyboard activation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Input {
    /// Native pointer button, if supplied.
    pub button: Option<Button>,
    /// True for known keyboard activation, false for known pointer activation.
    pub keyboard: Option<bool>,
    /// Native click count, if supplied.
    pub click_count: Option<u32>,
    /// Opaque short-lived compositor activation token.
    pub activation_token: Option<String>,
}

/// Semantic native event with stable registration context.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum EventKind {
    /// Primary activation.
    Activate,
    /// Secondary activation, often a middle click on Linux.
    SecondaryActivate,
    /// Native context menu request.
    ContextMenu,
    /// Selection of an application menu item.
    MenuSelected(String),
    /// Native menu opening/preparation hint.
    MenuOpened,
    /// Native menu closed.
    MenuClosed,
    /// Native menu highlight changed.
    MenuHighlighted(Option<String>),
    /// Native scroll request; units remain backend-defined.
    Scroll {
        /// Native delta.
        delta: i32,
        /// Horizontal when true, vertical otherwise.
        horizontal: bool,
    },
    /// Host became available or unavailable.
    HostChanged(bool),
    /// Native user-removal/visibility observation.
    VisibilityChanged(bool),
    /// Native callback ingress overflowed; commands were not silently ignored.
    Overflow,
}

/// One event from a specific registration, never merely a reused string name.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// Application-stable tray ID.
    pub id: String,
    /// Unique registration generation, scoped to one host.
    pub generation: u64,
    /// Native menu projection revision, when the event belongs to a menu.
    pub menu_revision: Option<u64>,
    /// Native event meaning.
    pub kind: EventKind,
    /// Optional native geometry evidence.
    pub anchor: Option<Anchor>,
    /// Known input detail.
    pub input: Input,
}
