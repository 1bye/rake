# GPUI Web 0.3.11

This source is from the Apache-2.0 `gpui-pre-web` 0.3.11 release,
based on Zed revision `f816e1f9263e3a8ef63c4baaa02e1441ce1c6038`.

The local patch preserves selected text when deriving edits from the IME
textarea and reconciles the textarea after the editor rejects an edit.
Remove the Cargo patch when the pinned GPUI snapshot includes these fixes.

Run the standalone diff tests with
`rustc --edition 2024 --test src/ime_edit.rs -o /tmp/gpui-ime-edit && /tmp/gpui-ime-edit`.
