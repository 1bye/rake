use crate::{
    Anchor, Button, CoordinateSpace, Error, EventKind, Input, Point, Rect, Registration, Rendering,
    Request, Tray, host::Sink,
};
use objc2::runtime::NSObject;
use objc2::{
    AnyThread, DeclaredClass, MainThreadMarker, MainThreadOnly, define_class, msg_send,
    rc::Retained, sel,
};
use objc2_app_kit::{
    NSApplication, NSEventMask, NSEventType, NSImage, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength,
};
use objc2_foundation::{NSData, NSObjectProtocol, NSSize, NSString};
use std::{cell::RefCell, rc::Rc};

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Sink]
    struct ActionTarget;

    unsafe impl NSObjectProtocol for ActionTarget {}

    impl ActionTarget {
        #[unsafe(method(rakeTrayAction:))]
        fn invoke(&self, _sender: &NSObject) {
            let mtm = MainThreadMarker::from(self);
            let event = NSApplication::sharedApplication(mtm).currentEvent();
            let button = event.as_ref().and_then(|event| match event.r#type() {
                NSEventType::LeftMouseUp => Some(Button::Primary),
                NSEventType::RightMouseUp => Some(Button::Secondary),
                NSEventType::OtherMouseUp => Some(Button::Other(event.buttonNumber() as u16)),
                _ => None,
            });
            let kind = if button == Some(Button::Secondary) { EventKind::ContextMenu }
                else if matches!(button, Some(Button::Other(2))) { EventKind::SecondaryActivate }
                else { EventKind::Activate };
            let input = Input { button, keyboard: button.map(|_| false),
                click_count: event.as_ref().filter(|_| button.is_some()).map(|event| event.clickCount() as u32), ..Input::default() };
            self.ivars().send(kind, None, input);
        }
    }
);

pub(crate) struct Item {
    item: Retained<NSStatusItem>,
    _target: Retained<ActionTarget>,
    mtm: MainThreadMarker,
    removed: std::cell::Cell<bool>,
    revision: std::cell::Cell<u64>,
    image: RefCell<Option<Retained<NSImage>>>,
}

impl Item {
    pub(crate) fn check_thread() -> Result<(), Error> {
        MainThreadMarker::new().map(|_| ()).ok_or_else(|| {
            Error::InvalidInput("AppKit tray resources require the main thread".into())
        })
    }

    pub(crate) fn create(application: &str, spec: &Tray, sink: Sink) -> Request<Self> {
        crate::ready(Self::new(application, spec, sink))
    }

    fn new(application: &str, spec: &Tray, sink: Sink) -> Result<Self, Error> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            Error::InvalidInput("AppKit tray creation requires the main thread".into())
        })?;
        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        item.setAutosaveName(Some(&NSString::from_str(&format!(
            "{application}.{}",
            spec.id
        ))));
        let allocated = ActionTarget::alloc(mtm).set_ivars(sink);
        // SAFETY: ActionTarget is an NSObject subclass with initialized ivars.
        let target: Retained<ActionTarget> = unsafe { msg_send![super(allocated), init] };
        let result = Self {
            item,
            _target: target,
            mtm,
            removed: std::cell::Cell::new(false),
            revision: std::cell::Cell::new(0),
            image: RefCell::new(None),
        };
        let button = result
            .item
            .button(mtm)
            .ok_or_else(|| Error::HostUnavailable)?;
        // SAFETY: The retained target implements this selector and outlives the button attachment.
        unsafe {
            button.setTarget(Some(&result._target));
            button.setAction(Some(sel!(rakeTrayAction:)));
        }
        button.sendActionOn(
            NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp | NSEventMask::OtherMouseUp,
        );
        result.apply_now(spec)?;

        Ok(result)
    }

    fn apply_now(&self, spec: &Tray) -> Result<(), Error> {
        let button = self.item.button(self.mtm).ok_or(Error::HostUnavailable)?;
        let image = if let Some(source) = &spec.image {
            let named = source.native_name().and_then(|name| {
                NSImage::imageNamed(&NSString::from_str(name)).or_else(|| {
                    NSImage::imageWithSystemSymbolName_accessibilityDescription(
                        &NSString::from_str(name),
                        Some(&NSString::from_str(&spec.tooltip)),
                    )
                })
            });
            let image = if let Some(named) = named {
                named
            } else {
                NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(&source.to_png()?))
                    .ok_or(Error::HostUnavailable)?
            };
            // Copy before changing native shared image metadata.
            use objc2_foundation::NSCopying;
            let image = image.copy();
            image.setSize(NSSize::new(18.0, 18.0));
            image.setTemplate(source.rendering == Rendering::Template);

            Some(image)
        } else {
            None
        };
        button.setImage(image.as_deref());
        button.setToolTip(Some(&NSString::from_str(&spec.tooltip)));
        *self.image.borrow_mut() = image;
        self.item.setVisible(spec.visible);

        Ok(())
    }

    pub(crate) fn apply(self: &Rc<Self>, revision: u64, spec: &Tray) -> Request<()> {
        let item = self.clone();
        let spec = spec.clone();
        Box::pin(async move {
            if item.removed.get() {
                return Err(Error::Removed);
            }
            if item.revision.get() <= revision {
                item.apply_now(&spec)?;
                item.revision.set(revision);
            }
            Ok(())
        })
    }
    pub(crate) fn registration(&self) -> Registration {
        Registration::Registered
    }
    pub(crate) fn anchor(&self) -> Result<Option<Anchor>, Error> {
        let Some(button) = self.item.button(self.mtm) else {
            return Ok(None);
        };
        let Some(window) = button.window() else {
            return Ok(None);
        };
        let rect = window.convertRectToScreen(button.convertRect_toView(button.bounds(), None));

        Ok(Some(Anchor {
            rect: Some(Rect {
                origin: Point {
                    x: rect.origin.x,
                    y: rect.origin.y,
                },
                width: rect.size.width,
                height: rect.size.height,
            }),
            point: None,
            space: CoordinateSpace::AppKitScreen,
        }))
    }

    pub(crate) fn shutdown(&self) -> Request<()> {
        self.remove();
        crate::ready(Ok(()))
    }

    fn remove(&self) {
        if !self.removed.replace(true) {
            if let Some(button) = self.item.button(self.mtm) {
                // SAFETY: Clear the weak target before releasing its retained owner.
                unsafe {
                    button.setTarget(None);
                    button.setAction(None);
                }
            }

            NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
        }
    }
}

impl Drop for Item {
    fn drop(&mut self) {
        self.remove();
    }
}
