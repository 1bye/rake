use crate::{Anchor, Error, Feature, Registration, Request, Tray, host::Sink};

pub(crate) struct Item;

impl Item {
    pub(crate) fn check_thread() -> Result<(), Error> {
        Err(Error::Unsupported(Feature::MultipleItems))
    }
    pub(crate) fn create(_: &str, _: &Tray, _: Sink) -> Request<Self> {
        crate::ready(Err(Error::Unsupported(Feature::MultipleItems)))
    }
    pub(crate) fn apply(&self, _: u64, _: &Tray) -> Request<()> {
        crate::ready(Err(Error::Unsupported(Feature::MultipleItems)))
    }
    pub(crate) fn show_menu(&self) -> Result<(), Error> {
        Err(Error::Unsupported(Feature::ShowMenu))
    }
    pub(crate) fn anchor(&self) -> Result<Option<Anchor>, Error> {
        Err(Error::Unsupported(Feature::Rectangle))
    }
    pub(crate) fn registration(&self) -> Registration {
        Registration::WaitingForHost
    }
    pub(crate) fn popup_anchor(&self) -> Result<Option<crate::PopupAnchor>, Error> {
        Ok(None)
    }
    pub(crate) fn shutdown(&self) -> Request<()> {
        crate::ready(Ok(()))
    }
}
