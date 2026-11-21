use rake_protocol::router::RouteAddress;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Route {
    #[default]
    Overview,
    Activity,
    Settings,
}

impl Route {
    pub(crate) const ALL: [Self; 3] = [Self::Overview, Self::Activity, Self::Settings];

    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Activity => "activity",
            Self::Settings => "settings",
        }
    }

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Activity => "Activity",
            Self::Settings => "Settings",
        }
    }

    pub(crate) fn address(self) -> RouteAddress {
        RouteAddress {
            route: self.key().into(),
            params: Default::default(),
        }
    }
}

#[derive(Default)]
pub(crate) struct Navigation {
    current: Route,
}

impl Navigation {
    pub(crate) fn current(&self) -> Route {
        self.current
    }

    pub(crate) fn open(&mut self, route: Route) -> Option<RouteAddress> {
        if self.current == route {
            return None;
        }

        self.current = route;
        Some(route.address())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_updates_the_destination_and_emits_its_protocol_address() {
        let mut nav = Navigation::default();
        assert_eq!(nav.current(), Route::Overview);
        for (route, key) in [
            (Route::Settings, "settings"),
            (Route::Activity, "activity"),
            (Route::Overview, "overview"),
        ] {
            let address = nav.open(route).unwrap();
            assert_eq!(address.route, key);
            assert!(address.params.is_empty());
            assert_eq!(nav.current(), route);
            assert_eq!(nav.open(route), None);
        }
    }
}
