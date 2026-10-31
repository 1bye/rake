//! Internal retention for keyed navigation status and retry actions.

use crate::router::{NavigationStatus, Route, WindowId};

use super::Action;

pub(crate) struct Record<R: Route> {
    pub(crate) status: NavigationStatus,
    pub(crate) action: Option<Action<R>>,
    pub(crate) origin_window: WindowId,
}
