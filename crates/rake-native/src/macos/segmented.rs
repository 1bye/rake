//! Native single-selection segmented controls.

use std::{cell::RefCell, rc::Rc};

use gpui::App;
use objc2::{MainThreadMarker, rc::Retained, sel};
use objc2_app_kit::{NSSegmentStyle, NSSegmentSwitchTracking, NSSegmentedControl, NSView};
use objc2_foundation::NSString;

use super::{NativeElement, NativeError, NativeView, action::ActionTarget};

/// A native single-selection control with a fixed set of text segments.
/// Clones share selection and the action handler.
#[derive(Clone)]
pub struct SegmentedControl {
    view: NativeView,
    control: Retained<NSSegmentedControl>,
    target: Retained<ActionTarget>,
    count: usize,
}

impl SegmentedControl {
    /// Creates a control with its first segment selected.
    ///
    /// # Errors
    /// Returns [`NativeError::NotMainThread`] off the main thread, or
    /// [`NativeError::EmptySegments`] when `labels` is empty.
    pub fn new(labels: &[&str]) -> Result<Self, NativeError> {
        let mtm = MainThreadMarker::new().ok_or(NativeError::NotMainThread)?;

        if labels.is_empty() {
            return Err(NativeError::EmptySegments);
        }

        let control = NSSegmentedControl::new(mtm);

        // A valid slice of references cannot exceed isize::MAX elements.
        control.setSegmentCount(labels.len() as isize);
        control.setTrackingMode(NSSegmentSwitchTracking::SelectOne);
        control.setSegmentStyle(NSSegmentStyle::Automatic);

        for (index, label) in labels.iter().enumerate() {
            control.setLabel_forSegment(&NSString::from_str(label), index as isize);
        }

        control.setSelectedSegment(0);
        let target = ActionTarget::new(mtm);

        // SAFETY: The target implements this selector and is retained by NativeView.
        unsafe {
            control.setTarget(Some(&target));
            control.setAction(Some(sel!(rakeNativeAction:)));
        }
        let native: Retained<NSView> = control.clone().into_super().into_super();

        let view = NativeView::new(
            native.clone(),
            native,
            Some((control.clone().into_super(), target.clone())),
        )?;

        Ok(Self {
            view,
            control,
            target,
            count: labels.len(),
        })
    }

    /// Creates this control's GPUI layout slot.
    pub fn element(&self) -> NativeElement {
        self.view.element()
    }

    /// Underlying AppKit control for additional native configuration.
    /// Leave its target/action and segment count under this adapter's control.
    pub fn as_appkit(&self) -> &NSSegmentedControl {
        &self.control
    }

    /// Returns the selected zero-based index, or `None` if selection was cleared.
    pub fn selected(&self) -> Option<usize> {
        usize::try_from(self.control.selectedSegment()).ok()
    }

    /// Changes selection without invoking the user-action callback.
    ///
    /// # Errors
    /// Returns [`NativeError::InvalidSegment`] without changing the selection
    /// when the index lies outside this control. `None` clears the selection.
    pub fn set_selected(&self, index: Option<usize>) -> Result<(), NativeError> {
        if let Some(index) = index
            && index >= self.count
        {
            return Err(NativeError::InvalidSegment {
                index,
                count: self.count,
            });
        }

        self.control
            .setSelectedSegment(index.map_or(-1, |index| index as isize));

        Ok(())
    }

    /// Replaces the selection handler, scheduled on GPUI's foreground executor.
    /// The index is captured when AppKit sends the action. Capture weak entity
    /// handles when updating the GPUI view that owns this control.
    pub fn on_change(&self, cx: &App, handler: impl FnMut(usize, &mut App) + 'static) {
        let app = cx.to_async();
        let control = self.control.clone();
        let handler = Rc::new(RefCell::new(handler));

        self.target.set_callback(move || {
            let Ok(index) = usize::try_from(control.selectedSegment()) else {
                return;
            };

            let handler = handler.clone();

            app.spawn(async move |cx| {
                cx.update(|cx| (handler.borrow_mut())(index, cx));
            })
            .detach();
        });
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

impl std::fmt::Debug for SegmentedControl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SegmentedControl")
            .field("view", &self.view)
            .field("count", &self.count)
            .finish_non_exhaustive()
    }
}
