//! Fieldnotes keyboard compatibility with Base 0.9.

use rake::gpui::{self, KeyBinding};

struct KeyBindings;

impl gpui::Global for KeyBindings {}

pub(super) fn bind(cx: &mut gpui::App) {
    if cx.has_global::<KeyBindings>() {
        return;
    }

    cx.set_global(KeyBindings);
    const INPUT: Option<&str> = Some("Fieldnotes > Input");

    // Give Tab back to the workspace instead of indenting the body.
    cx.bind_keys([
        KeyBinding::new("tab", gpui::NoAction, INPUT),
        KeyBinding::new("shift-tab", gpui::NoAction, INPUT),
    ]);

    #[cfg(target_family = "wasm")]
    {
        use gpui_base::input::{Copy, Cut, Redo, SelectAll, Undo};

        // GPUI Web receives clipboard text from the browser's paste event.
        // Preserve the native shortcut on macOS browsers as well as Ctrl on Wasm.
        cx.bind_keys([
            KeyBinding::new("ctrl-v", gpui::NoAction, INPUT),
            KeyBinding::new("cmd-a", SelectAll, INPUT),
            KeyBinding::new("cmd-c", Copy, INPUT),
            KeyBinding::new("cmd-x", Cut, INPUT),
            KeyBinding::new("cmd-z", Undo, INPUT),
            KeyBinding::new("cmd-shift-z", Redo, INPUT),
            KeyBinding::new("ctrl-shift-z", Redo, INPUT),
        ]);
    }
}

#[cfg(all(test, feature = "view-test-support"))]
mod tests;
