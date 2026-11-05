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
    NSAccessibility, NSApplication, NSCellImagePosition, NSEventMask, NSEventType, NSImage,
    NSSquareStatusItemLength, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSData, NSObjectProtocol, NSSize, NSString};
use objc2_foundation::{NSKeyValueObservingOptions, NSObjectNSKeyValueObserverRegistration};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RakeTrayActionTarget"]
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
                NSEventType::OtherMouseUp if event.buttonNumber() == 2 => Some(Button::Middle),
                NSEventType::OtherMouseUp => Some(Button::Other(event.buttonNumber() as u16)),
                _ => None,
            });
            let kind = if button == Some(Button::Secondary) { EventKind::ContextMenu }
                else if button == Some(Button::Middle) { EventKind::SecondaryActivate }
                else { EventKind::Activate };
            let modifiers = event.as_ref().map(|event| {
                let flags = event.modifierFlags();
                use objc2_app_kit::NSEventModifierFlags as Flags;
                crate::Modifiers { shift: flags.contains(Flags::Shift), control: flags.contains(Flags::Control), alt: flags.contains(Flags::Option), super_key: flags.contains(Flags::Command) }
            });
            let timestamp = event.as_ref().and_then(|event| std::time::Duration::try_from_secs_f64(event.timestamp()).ok());
            let keyboard = event.as_ref().and_then(|event| match event.r#type() { NSEventType::KeyDown | NSEventType::KeyUp => Some(true), _ => button.map(|_| false) });
            let input = Input { button, keyboard, modifiers, timestamp,
                click_count: event.as_ref().filter(|_| button.is_some()).map(|event| event.clickCount() as u32), ..Input::default() };
            self.ivars().send(kind, None, input);
        }
    }
);

struct VisibilityState {
    sink: Sink,
    programmatic: Rc<Cell<bool>>,
    removal: Rc<Cell<crate::macos::Removal>>,
}
define_class!(
    // SAFETY: The observer is retained and detached before its item owner is dropped.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RakeTrayVisibilityObserver"]
    #[ivars = VisibilityState]
    struct VisibilityObserver;
    // SAFETY: NSObjectProtocol has no additional invariants.
    unsafe impl NSObjectProtocol for VisibilityObserver {}
    impl VisibilityObserver {
        // SAFETY: Only registered for NSStatusItem.visible; object is our retained item.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn changed(&self, _: &NSString, object: &NSStatusItem, _: *const objc2::runtime::AnyObject, _: *mut std::ffi::c_void) {
            let visible = object.isVisible();
            let state = self.ivars();
            if !state.programmatic.get() { state.sink.send(EventKind::VisibilityChanged(visible), None, Input::default()); }
            if !visible && !state.programmatic.get() && state.removal.get() == crate::macos::Removal::RequestQuit {
                state.sink.send(EventKind::RemovalRequestedQuit, None, Input::default());
            }
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
    menu: RefCell<Option<Rc<super::macos_menu::NativeMenu>>>,
    sink: Sink,
    observer: Retained<VisibilityObserver>,
    programmatic: Rc<Cell<bool>>,
    removal: Rc<Cell<crate::macos::Removal>>,
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
        let allocated = ActionTarget::alloc(mtm).set_ivars(sink.clone());
        // SAFETY: ActionTarget is an NSObject subclass with initialized ivars.
        let target: Retained<ActionTarget> = unsafe { msg_send![super(allocated), init] };
        let programmatic = Rc::new(Cell::new(false));
        let removal = Rc::new(Cell::new(crate::macos::Removal::Prevent));
        let allocated = VisibilityObserver::alloc(mtm).set_ivars(VisibilityState {
            sink: sink.clone(),
            programmatic: programmatic.clone(),
            removal: removal.clone(),
        });
        // SAFETY: NSObject subclass allocation with initialized ivars.
        let observer: Retained<VisibilityObserver> = unsafe { msg_send![super(allocated), init] };
        // SAFETY: This dedicated observer is retained until the matching removal in Item::remove.
        unsafe {
            item.addObserver_forKeyPath_options_context(
                &observer,
                &NSString::from_str("visible"),
                NSKeyValueObservingOptions::New,
                std::ptr::null_mut(),
            );
        }
        let result = Self {
            observer,
            programmatic,
            removal,
            item,
            _target: target,
            mtm,
            removed: std::cell::Cell::new(false),
            revision: std::cell::Cell::new(0),
            image: RefCell::new(None),
            menu: RefCell::new(None),
            sink,
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
        let mut initial = spec.clone();
        if initial
            .macos
            .as_ref()
            .is_some_and(|mac| mac.removal != crate::macos::Removal::Prevent)
        {
            initial.visible &= result.item.isVisible();
        }
        result.apply_now(0, &initial)?;

        Ok(result)
    }

    fn apply_now(&self, revision: u64, spec: &Tray) -> Result<(), Error> {
        let button = self.item.button(self.mtm).ok_or(Error::HostUnavailable)?;
        let image = spec
            .image
            .as_ref()
            .map(|source| native_image(source, &spec.tooltip))
            .transpose()?;
        let menu = spec
            .menu
            .as_ref()
            .map(|menu| {
                super::macos_menu::NativeMenu::new(menu, self.sink.menu(revision), self.mtm)
                    .map(Rc::new)
            })
            .transpose()?;
        let appearance = spec.macos.clone().unwrap_or_default();
        let alternate = appearance
            .alternate_image
            .as_ref()
            .map(|image| native_image(image, &appearance.accessibility_label))
            .transpose()?;
        let attributed = appearance
            .attributed_title
            .as_ref()
            .map(attributed_text)
            .transpose()?;
        let alternate_attributed = appearance
            .attributed_alternate_title
            .as_ref()
            .map(attributed_text)
            .transpose()?;
        if spec.interaction == crate::Interaction::NativeMenu {
            self.item.setMenu(menu.as_ref().map(|m| &*m.menu));
        } else {
            self.item.setMenu(None);
        }
        *self.menu.borrow_mut() = menu;
        self.item.setLength(match appearance.length {
            crate::macos::ItemLength::Content => NSVariableStatusItemLength,
            crate::macos::ItemLength::Square => NSSquareStatusItemLength,
            crate::macos::ItemLength::Points(value) => value,
        });
        button.setTitle(&NSString::from_str(&appearance.title));
        button.setAlternateTitle(&NSString::from_str(&appearance.alternate_title));
        if let Some(text) = attributed {
            button.setAttributedTitle(&text);
        }
        if let Some(text) = alternate_attributed {
            button.setAttributedAlternateTitle(&text);
        }
        button.setAlternateImage(alternate.as_deref());
        button.setImagePosition(NSCellImagePosition::ImageLeading);
        button.setAppearsDisabled(appearance.appears_disabled);
        button.setEnabled(!appearance.disabled);
        button.setAccessibilityLabel(Some(&NSString::from_str(
            if appearance.accessibility_label.is_empty() {
                &spec.tooltip
            } else {
                &appearance.accessibility_label
            },
        )));
        button.setAccessibilityHelp(Some(&NSString::from_str(&appearance.accessibility_help)));
        button.setImage(image.as_deref());
        button.setToolTip(Some(&NSString::from_str(&spec.tooltip)));
        *self.image.borrow_mut() = image;
        self.removal.set(appearance.removal);
        self.item
            .setBehavior(if appearance.removal == crate::macos::Removal::Prevent {
                objc2_app_kit::NSStatusItemBehavior::empty()
            } else {
                objc2_app_kit::NSStatusItemBehavior::RemovalAllowed
            });
        self.programmatic.set(true);
        self.item.setVisible(spec.visible);
        self.programmatic.set(false);

        Ok(())
    }

    pub(crate) fn apply(self: &Rc<Self>, revision: u64, spec: &Tray) -> Request<()> {
        let item = self.clone();
        let spec = spec.clone();
        Box::pin(async move {
            if item.removed.get() {
                return Err(Error::Removed);
            }
            loop {
                let menu = item.menu.borrow().clone();
                let Some(menu) = menu else { break };
                let changed = menu.changed.listen();
                if !menu.tracking.get() {
                    break;
                }
                changed.await;
                if item.removed.get() {
                    return Err(Error::Removed);
                }
            }
            if item.revision.get() <= revision {
                item.apply_now(revision, &spec)?;
                item.revision.set(revision);
            }
            Ok(())
        })
    }
    #[cfg(feature = "native-tests")]
    pub(crate) fn probe_menu_selection(&self, index: usize) -> Result<(), Error> {
        let menu = self
            .menu
            .borrow()
            .clone()
            .ok_or(Error::InvalidInput("item has no native menu".into()))?;
        if index >= menu.menu.numberOfItems() as usize {
            return Err(Error::InvalidInput("menu probe index is invalid".into()));
        }
        menu.menu.performActionForItemAtIndex(index as isize);
        Ok(())
    }

    pub(crate) fn show_menu(&self) -> Result<(), Error> {
        let menu = self
            .menu
            .borrow()
            .clone()
            .ok_or(Error::InvalidInput("item has no menu".into()))?;
        let button = self.item.button(self.mtm).ok_or(Error::Removed)?;
        menu.menu.popUpMenuPositioningItem_atLocation_inView(
            None,
            objc2_foundation::NSPoint::new(0.0, button.bounds().size.height),
            Some(&button),
        );
        Ok(())
    }

    pub(crate) fn metrics(&self) -> crate::macos::Metrics {
        let bar = NSStatusBar::systemStatusBar();
        crate::macos::Metrics {
            vertical: bar.isVertical(),
            thickness: bar.thickness(),
            length: self.item.length(),
            visible: self.item.isVisible(),
        }
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

    pub(crate) fn set_popup_highlight(&self, highlighted: bool) -> Result<(), Error> {
        self.item
            .button(self.mtm)
            .ok_or(Error::Removed)?
            .highlight(highlighted);
        Ok(())
    }

    pub(crate) fn popup_anchor(&self) -> Result<Option<crate::PopupAnchor>, Error> {
        let Some(button) = self.item.button(self.mtm) else {
            return Ok(None);
        };
        let Some(window) = button.window() else {
            return Ok(None);
        };
        let Some(screen) = window.screen() else {
            return Ok(None);
        };
        let Some(number) = screen
            .deviceDescription()
            .objectForKey(&NSString::from_str("NSScreenNumber"))
        else {
            return Ok(None);
        };
        // SAFETY: NSScreenNumber is documented as an NSNumber in the device description.
        let id: u32 = unsafe { msg_send![&*number, unsignedIntValue] };
        let frame = screen.frame();
        let convert = |rect: objc2_foundation::NSRect| Rect {
            origin: Point {
                x: rect.origin.x - frame.origin.x,
                y: frame.origin.y + frame.size.height - rect.origin.y - rect.size.height,
            },
            width: rect.size.width,
            height: rect.size.height,
        };
        let icon = window.convertRectToScreen(button.convertRect_toView(button.bounds(), None));
        Ok(Some(crate::PopupAnchor {
            display_id: id.into(),
            icon: convert(icon),
            work_area: convert(screen.visibleFrame()),
        }))
    }

    pub(crate) fn shutdown(&self) -> Request<()> {
        self.remove();
        crate::ready(Ok(()))
    }

    fn remove(&self) {
        if !self.removed.replace(true) {
            if let Some(menu) = self.menu.borrow().as_ref() {
                menu.menu.cancelTracking();
            }
            // SAFETY: This is the exact observer/key pair registered during creation.
            unsafe {
                self.item
                    .removeObserver_forKeyPath(&self.observer, &NSString::from_str("visible"));
            }
            self.item.setMenu(None);
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

pub(super) fn native_image(
    source: &crate::Image,
    description: &str,
) -> Result<Retained<NSImage>, Error> {
    let named = source.native_name().and_then(|name| {
        NSImage::imageNamed(&NSString::from_str(name)).or_else(|| {
            NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(name),
                Some(&NSString::from_str(description)),
            )
        })
    });
    let image = if let Some(named) = named {
        named
    } else {
        if source.representations.is_empty() {
            return Err(Error::Unsupported(crate::Feature::NamedImage));
        }
        let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(18.0, 18.0));
        for rep in &source.representations {
            let bitmap = objc2_app_kit::NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(
                &rep.to_png()?,
            ))
            .ok_or(Error::HostUnavailable)?;
            bitmap.setSize(NSSize::new(18.0, 18.0));
            image.addRepresentation(&bitmap);
        }
        image
    };
    use objc2_foundation::NSCopying;
    let image = image.copy();
    image.setSize(NSSize::new(18.0, 18.0));
    image.setTemplate(source.rendering == Rendering::Template);
    Ok(image)
}

pub(super) fn attributed_text(
    text: &crate::macos::AttributedText,
) -> Result<Retained<objc2_foundation::NSAttributedString>, Error> {
    use objc2::runtime::AnyObject;
    use objc2_app_kit::{NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName};
    use objc2_foundation::{NSAttributedString, NSDictionary, NSMutableAttributedString};
    let result = NSMutableAttributedString::new();
    for run in &text.runs {
        let font = if run.bold {
            NSFont::boldSystemFontOfSize(run.size.into())
        } else {
            NSFont::systemFontOfSize(run.size.into())
        };
        let color = run.color.map(|rgba| {
            NSColor::colorWithSRGBRed_green_blue_alpha(
                f64::from(rgba[0]) / 255.0,
                f64::from(rgba[1]) / 255.0,
                f64::from(rgba[2]) / 255.0,
                f64::from(rgba[3]) / 255.0,
            )
        });
        // SAFETY: Static AppKit attribute keys; paired values have the documented NSFont/NSColor types.
        let attributes: Retained<NSDictionary<NSString, AnyObject>> = unsafe {
            let mut keys = vec![NSFontAttributeName];
            let mut values: Vec<&AnyObject> = vec![&font];
            if let Some(color) = &color {
                keys.push(NSForegroundColorAttributeName);
                values.push(color);
            }
            NSDictionary::from_slices(&keys, &values)
        };
        // SAFETY: Attribute keys and values above match AppKit's documented attributed-string contract.
        let part = unsafe {
            NSAttributedString::initWithString_attributes(
                NSAttributedString::alloc(),
                &NSString::from_str(&run.text),
                Some(&attributes),
            )
        };
        result.appendAttributedString(&part);
    }
    Ok(result.into_super())
}
