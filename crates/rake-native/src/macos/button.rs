//! Native push and SF Symbol buttons with optional system glass.

use std::{cell::RefCell, rc::Rc};

use gpui::App;
use objc2::{MainThreadMarker, rc::Retained, runtime::AnyClass, sel};

use objc2_app_kit::{
    NSBezelStyle, NSButton, NSButtonType, NSGlassEffectView, NSGlassEffectViewStyle, NSImage,
    NSView,
};

use objc2_foundation::NSString;

use super::{NativeElement, NativeError, NativeView, action::ActionTarget};

/// Native button appearance. Liquid Glass falls back to a toolbar button on
/// macOS versions without `NSGlassEffectView` (before macOS 26).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ButtonStyle {
    /// Standard AppKit push-button appearance.
    #[default]
    Push,
    /// Standard AppKit toolbar-button appearance.
    Toolbar,
    /// A native button without its own bezel.
    Borderless,
    /// A borderless native button hosted inside a system glass view.
    LiquidGlass,
}

/// A retained AppKit button. Clones share the control and its action handler.
#[derive(Clone)]
pub struct Button {
    view: NativeView,
    control: Retained<NSButton>,
    target: Retained<ActionTarget>,
}

impl Button {
    /// Creates a text button on the main thread.
    ///
    /// # Errors
    /// Returns [`NativeError::NotMainThread`] off the application's main thread.
    pub fn new(title: &str, style: ButtonStyle) -> Result<Self, NativeError> {
        let mtm = MainThreadMarker::new().ok_or(NativeError::NotMainThread)?;
        let control = NSButton::new(mtm);

        control.setTitle(&NSString::from_str(title));
        control.setButtonType(NSButtonType::MomentaryPushIn);
        control.setBezelStyle(match style {
            ButtonStyle::Push => NSBezelStyle::Push,
            _ => NSBezelStyle::Toolbar,
        });
        control.setBordered(style != ButtonStyle::Borderless);
        let target = ActionTarget::new(mtm);

        // SAFETY: The retained ActionTarget implements this exact action selector
        // and NativeView keeps it alive for the native control's lifetime.
        unsafe {
            control.setTarget(Some(&target));
            control.setAction(Some(sel!(rakeNativeAction:)));
        }
        let measure: Retained<NSView> = control.clone().into_super().into_super();

        let root = if style == ButtonStyle::LiquidGlass && Self::glass_available() {
            control.setBordered(false);
            let glass = NSGlassEffectView::new(mtm);

            glass.setStyle(NSGlassEffectViewStyle::Regular);
            glass.setCornerRadius(8.0);
            glass.setContentView(Some(&measure));

            glass.into_super()
        } else {
            measure.clone()
        };

        let view = NativeView::new(
            root,
            measure,
            Some((control.clone().into_super(), target.clone())),
        )?;

        Ok(Self {
            view,
            control,
            target,
        })
    }

    /// Creates an icon button from an SF Symbol and its accessible description.
    ///
    /// # Errors
    /// Returns [`NativeError::NotMainThread`] off the main thread, or
    /// [`NativeError::SymbolUnavailable`] if this OS does not provide the symbol.
    pub fn symbol(name: &str, description: &str, style: ButtonStyle) -> Result<Self, NativeError> {
        let button = Self::new("", style)?;

        let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str(name),
            Some(&NSString::from_str(description)),
        )
        .ok_or_else(|| NativeError::SymbolUnavailable(name.to_owned()))?;

        button.control.setImage(Some(&image));
        button
            .control
            .setToolTip(Some(&NSString::from_str(description)));

        Ok(button)
    }

    /// Whether this macOS runtime provides native Liquid Glass.
    pub fn glass_available() -> bool {
        AnyClass::get(c"NSGlassEffectView").is_some()
    }

    /// Creates this button's GPUI layout slot. Keep the button in your entity.
    pub fn element(&self) -> NativeElement {
        self.view.element()
    }

    /// Underlying AppKit control for additional native configuration.
    /// Leave its target/action and view attachment under this adapter's control.
    pub fn as_appkit(&self) -> &NSButton {
        &self.control
    }

    /// Replaces the click handler. Dispatch runs later on GPUI's foreground
    /// executor, outside AppKit's action stack. Capture weak GPUI entities to
    /// avoid a cycle between an owning entity and its button's callback.
    pub fn on_click(&self, cx: &App, handler: impl FnMut(&mut App) + 'static) {
        let app = cx.to_async();
        let handler = Rc::new(RefCell::new(handler));

        self.target.set_callback(move || {
            let handler = handler.clone();

            app.spawn(async move |cx| {
                cx.update(|cx| (handler.borrow_mut())(cx));
            })
            .detach();
        });
    }

    /// Updates the title; notify the owning GPUI entity to recompute layout.
    pub fn set_title(&self, title: &str) {
        self.control.setTitle(&NSString::from_str(title));
    }

    /// Allows native mouse and keyboard activation.
    pub fn enable(&self) {
        self.control.setEnabled(true);
    }

    /// Prevents activation and applies AppKit's disabled appearance.
    pub fn disable(&self) {
        self.control.setEnabled(false);
    }
}

impl std::fmt::Debug for Button {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Button")
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}
