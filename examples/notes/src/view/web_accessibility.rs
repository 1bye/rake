//! Projects Notes editor semantics into the browser accessibility tree.

use super::{BODY_LABEL, NotesView, SEARCH_LABEL, TITLE_LABEL};
use rake::gpui::{Context, FocusHandle, Window};
use wasm_bindgen::{JsCast, prelude::*};

#[wasm_bindgen(module = "/src/view/web_accessibility.js")]
extern "C" {
    type BrowserFields;

    #[wasm_bindgen(constructor, catch)]
    fn new(
        search: &str,
        title: &str,
        body: &str,
        focus_field: &js_sys::Function,
    ) -> Result<BrowserFields, JsValue>;

    #[wasm_bindgen(method)]
    fn sync(this: &BrowserFields, ready: bool, has_note: bool, active: Option<usize>);

    #[wasm_bindgen(method)]
    fn remove(this: &BrowserFields);
}

pub(super) struct WebAccessibility {
    fields: BrowserFields,
    handles: [FocusHandle; 3],
    _focus_field: Closure<dyn FnMut(usize) -> bool>,
}

impl WebAccessibility {
    pub(super) fn new(
        handles: [FocusHandle; 3],
        window: &mut Window,
        cx: &mut Context<NotesView>,
    ) -> Option<Self> {
        let mut context = window.to_async(cx);
        let focus_handles = handles.clone();
        let focus_field = Closure::new(move |index: usize| {
            let Some(handle) = focus_handles.get(index) else {
                return false;
            };

            context
                .update(|window, cx| window.focus(handle, cx))
                .is_ok()
        });

        let fields = match BrowserFields::new(
            SEARCH_LABEL,
            TITLE_LABEL,
            BODY_LABEL,
            focus_field.as_ref().unchecked_ref(),
        ) {
            Ok(fields) => fields,
            Err(error) => {
                rake::warn!(?error, "Could not expose Notes fields to the browser");

                return None;
            }
        };

        for handle in &handles {
            cx.on_focus(handle, window, |_, _, cx| cx.notify()).detach();
            cx.on_blur(handle, window, |_, _, cx| cx.notify()).detach();
        }

        Some(Self {
            fields,
            handles,
            _focus_field: focus_field,
        })
    }

    pub(super) fn sync(&self, window: &Window, ready: bool, has_note: bool) {
        let active = self
            .handles
            .iter()
            .position(|handle| handle.is_focused(window));

        self.fields.sync(ready, has_note, active);
    }
}

impl Drop for WebAccessibility {
    fn drop(&mut self) {
        self.fields.remove();
    }
}
