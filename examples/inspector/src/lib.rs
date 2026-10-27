//! Elements-only developer tools built with Rake on native and browser hosts.
//! The UI observes another runtime through its authenticated local inspector broker.

#[cfg(feature = "gui")]
mod appearance;
mod connection;
#[cfg(feature = "gui")]
mod input;
mod model;
#[cfg(feature = "gui")]
mod styles;
#[cfg(feature = "gui")]
mod tree;
#[cfg(feature = "gui")]
mod view;

pub use connection::{Connection, ConnectionConfig, Instance};
pub use model::{Inspector, Update};

#[cfg(feature = "gui")]
#[rake::app]
fn app() -> rake::App<view::Destination> {
    view::application()
}
