use crate::{CheckState, Error, EventKind, Input, ItemKind, Menu, host::Sink};

use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject, sel,
};

use objc2_app_kit::{NSEventModifierFlags, NSMenu, NSMenuDelegate, NSMenuItem};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
use std::{cell::Cell, rc::Rc};

struct Action {
    sink: Sink,
    id: String,
}

define_class!(
    // SAFETY: NSObject subclass with main-thread-only owned ivars and no custom deallocation.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "RakeTrayMenuAction"]
    #[ivars = Action]
    struct Target;
    // SAFETY: NSObjectProtocol has no additional invariants.
    unsafe impl NSObjectProtocol for Target {}
    impl Target {
        // SAFETY: Implements AppKit target/action's object-argument ABI.
        #[unsafe(method(rakeTrayMenuAction:))]
        fn invoke(&self, _sender: &NSObject) {
            self.ivars().sink.send(EventKind::MenuSelected(self.ivars().id.clone()), None, Input::default());
        }
    }
);

struct Observation {
    sink: Sink,
    tracking: Rc<Cell<bool>>,
    changed: Rc<event_listener::Event>,
}

define_class!(
    // SAFETY: Delegate is retained by NativeMenu, stays on the main thread and owns its state.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "RakeTrayMenuDelegate"]
    #[ivars = Observation]
    struct Observer;
    // SAFETY: NSObjectProtocol has no additional invariants.
    unsafe impl NSObjectProtocol for Observer {}
    // SAFETY: Methods match NSMenuDelegate signatures and only enqueue internal events.
    unsafe impl NSMenuDelegate for Observer {
        #[unsafe(method(menuWillOpen:))]
        fn will_open(&self, _: &NSMenu) {
            self.ivars().tracking.set(true);
            self.ivars()
                .sink
                .send(EventKind::MenuOpened, None, Input::default());
        }
        #[unsafe(method(menu:willHighlightItem:))]
        fn highlighted(&self, _: &NSMenu, item: Option<&NSMenuItem>) {
            let id = item
                .and_then(|item| item.representedObject())
                .and_then(|value| value.downcast::<NSString>().ok())
                .map(|value| value.to_string());
            self.ivars()
                .sink
                .send(EventKind::MenuHighlighted(id), None, Input::default());
        }
        #[unsafe(method(menuDidClose:))]
        fn did_close(&self, _: &NSMenu) {
            self.ivars().tracking.set(false);
            self.ivars().changed.notify(usize::MAX);
            self.ivars()
                .sink
                .send(EventKind::MenuClosed, None, Input::default());
        }
    }
);

pub(super) struct NativeMenu {
    pub(super) menu: Retained<NSMenu>,
    _targets: Vec<Retained<Target>>,
    _observer: Retained<Observer>,
    pub(super) tracking: Rc<Cell<bool>>,
    pub(super) changed: Rc<event_listener::Event>,
}

impl NativeMenu {
    pub(super) fn new(spec: &Menu, sink: Sink, mtm: MainThreadMarker) -> Result<Self, Error> {
        let tracking = Rc::new(Cell::new(false));
        let changed = Rc::new(event_listener::Event::new());

        let allocated = Observer::alloc(mtm).set_ivars(Observation {
            sink: sink.clone(),
            tracking: tracking.clone(),
            changed: changed.clone(),
        });

        // SAFETY: Initialized NSObject subclass allocation.
        let observer: Retained<Observer> = unsafe { msg_send![super(allocated), init] };
        let mut targets = Vec::new();
        let menu = build(spec, &sink, mtm, &mut targets)?;

        menu.setDelegate(Some(ProtocolObject::from_ref(&*observer)));

        Ok(Self {
            menu,
            _targets: targets,
            _observer: observer,
            tracking,
            changed,
        })
    }
}

impl Drop for NativeMenu {
    fn drop(&mut self) {
        self.menu.setDelegate(None);
    }
}

fn build(
    spec: &Menu,
    sink: &Sink,
    mtm: MainThreadMarker,
    targets: &mut Vec<Retained<Target>>,
) -> Result<Retained<NSMenu>, Error> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(""));

    menu.setAutoenablesItems(false);

    if let Some(layout) = &spec.macos {
        menu.setMinimumWidth(layout.minimum_width.into());
        menu.setShowsStateColumn(layout.shows_state_column);
        menu.setAllowsContextMenuPlugIns(layout.context_plugins);

        if layout.palette {
            if !menu.respondsToSelector(sel!(setPresentationStyle:)) {
                return Err(Error::Unsupported(crate::Feature::MenuPalette));
            }

            menu.setPresentationStyle(objc2_app_kit::NSMenuPresentationStyle::Palette);
        }

        if layout.selection != crate::macos::Selection::Automatic {
            if !menu.respondsToSelector(sel!(setSelectionMode:)) {
                return Err(Error::Unsupported(crate::Feature::MenuPalette));
            }

            menu.setSelectionMode(match layout.selection {
                crate::macos::Selection::Automatic => objc2_app_kit::NSMenuSelectionMode::Automatic,
                crate::macos::Selection::One => objc2_app_kit::NSMenuSelectionMode::SelectOne,
                crate::macos::Selection::Any => objc2_app_kit::NSMenuSelectionMode::SelectAny,
            });
        }
    }

    for spec in &spec.items {
        let item = if matches!(spec.kind, ItemKind::Separator) {
            NSMenuItem::separatorItem(mtm)
        } else {
            // SAFETY: Selector is implemented by our retained Target; no target is installed yet.
            unsafe {
                NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(&spec.label),
                    Some(sel!(rakeTrayMenuAction:)),
                    &NSString::from_str(""),
                )
            }
        };

        if let Some(appearance) = &spec.macos {
            if let Some(badge) = &appearance.badge {
                if !item.respondsToSelector(sel!(setBadge:)) {
                    return Err(Error::Unsupported(crate::Feature::MenuBadges));
                }

                use crate::macos::Badge;
                use objc2::AnyThread as _;
                use objc2_app_kit::NSMenuItemBadge;
                let badge = match badge {
                    Badge::Count(count) => {
                        NSMenuItemBadge::initWithCount(NSMenuItemBadge::alloc(), *count as isize)
                    }
                    Badge::Text(text) => NSMenuItemBadge::initWithString(
                        NSMenuItemBadge::alloc(),
                        &NSString::from_str(text),
                    ),
                    Badge::NewItems(count) => NSMenuItemBadge::newItemsWithCount(*count as isize),
                    Badge::Updates(count) => NSMenuItemBadge::updatesWithCount(*count as isize),
                    Badge::Alerts(count) => NSMenuItemBadge::alertsWithCount(*count as isize),
                };

                item.setBadge(Some(&badge));
            }

            if let Some(images) = &appearance.state_images {
                let off = images
                    .off
                    .as_ref()
                    .map(|image| super::macos::native_image(image, &spec.label))
                    .transpose()?;

                let on = images
                    .on
                    .as_ref()
                    .map(|image| super::macos::native_image(image, &spec.label))
                    .transpose()?;

                let mixed = images
                    .mixed
                    .as_ref()
                    .map(|image| super::macos::native_image(image, &spec.label))
                    .transpose()?;

                item.setOffStateImage(off.as_deref());

                // SAFETY: NSMenuItem copies/retains validated image objects; pointers are not cached by us.
                unsafe {
                    item.setOnStateImage(on.as_deref());
                    item.setMixedStateImage(mixed.as_deref());
                }
            }

            if let Some(text) = &appearance.attributed_title {
                item.setAttributedTitle(Some(&*super::macos::attributed_text(text)?));
            }

            item.setAlternate(appearance.alternate);
            item.setIndentationLevel(appearance.indentation.into());
            item.setToolTip(
                appearance
                    .tooltip
                    .as_ref()
                    .map(|s| NSString::from_str(s))
                    .as_deref(),
            );

            if let Some(subtitle) = &appearance.subtitle {
                if !item.respondsToSelector(sel!(setSubtitle:)) {
                    return Err(Error::Unsupported(crate::Feature::MenuSubtitles));
                }

                item.setSubtitle(Some(&NSString::from_str(subtitle)));
            }
        }

        if let Some(value) = &spec.accelerator {
            let key = crate::Accelerator::parse(value)?;
            let mut modifiers = NSEventModifierFlags::empty();

            if key.control {
                modifiers |= NSEventModifierFlags::Control;
            }

            if key.alt {
                modifiers |= NSEventModifierFlags::Option;
            }

            if key.shift {
                modifiers |= NSEventModifierFlags::Shift;
            }

            if key.super_key {
                modifiers |= NSEventModifierFlags::Command;
            }

            item.setKeyEquivalent(&NSString::from_str(&key.key));
            item.setKeyEquivalentModifierMask(modifiers);
        }

        item.setHidden(!spec.visible);
        item.setEnabled(
            spec.enabled && !matches!(spec.kind, ItemKind::Header | ItemKind::Separator),
        );

        match &spec.kind {
            ItemKind::Submenu(submenu) => {
                item.setSubmenu(Some(&*build(submenu, sink, mtm, targets)?));
            }
            ItemKind::Check(state) => item.setState(match state {
                CheckState::Off => 0,
                CheckState::On => 1,
                CheckState::Mixed => -1,
            }),
            ItemKind::Radio { selected, .. } => item.setState(isize::from(*selected)),
            _ => {}
        }

        if let Some(image) = &spec.image {
            item.setImage(Some(&*super::macos::native_image(image, &spec.label)?));
        }

        let allocated = Target::alloc(mtm).set_ivars(Action {
            sink: sink.clone(),
            id: spec.id.clone(),
        });

        // SAFETY: NSObject subclass allocation with initialized ivars.
        let target: Retained<Target> = unsafe { msg_send![super(allocated), init] };

        // SAFETY: The selector and retained target have matching action ABI and outlive the menu.
        unsafe {
            item.setTarget(Some(&target));
        }
        targets.push(target);
        // SAFETY: Rake owns this private NSString payload; no user pointer is attached.
        unsafe {
            item.setRepresentedObject(Some(&NSString::from_str(&spec.id)));
        }
        menu.addItem(&item);
    }

    Ok(menu)
}
