//! Fieldnotes: a local-first reference application built with Rake.

#[cfg(feature = "gui")]
mod editor;
mod model;
mod persistence;
mod store;
#[cfg(feature = "gui")]
mod view;

pub use model::{Library, Note};
pub use persistence::{LoadReport, Repository, StorageError};
pub use store::{CreateNote, EditNote, ListNotes, NoteSummary, Notebook, SaveNotes};

#[cfg(feature = "gui")]
#[rake::app]
fn app() -> rake::App<view::Destination> {
    view::application()
}
