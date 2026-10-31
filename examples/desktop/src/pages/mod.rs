//! Typed navigation addresses; each registered page owns its construction.
mod home;
mod updates;

pub(crate) use home::Home;
pub(crate) use updates::Updates;

#[derive(Clone, rake::Route)]
pub(crate) enum Destination {
    #[route(id = "home", path = "/")]
    Home,
    #[route(id = "updates", path = "/updates")]
    Updates,
}
