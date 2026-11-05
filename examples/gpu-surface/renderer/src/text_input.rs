//! Native text state. Ranges at the controller boundary are UTF-16 offsets.

use std::ops::Range;

#[derive(Clone, Debug, Default)]
struct Buffer {
    text: String,
    selection: Range<usize>,
}

#[derive(Clone, Debug)]
struct Composition {
    before: Buffer,
    range: Range<usize>,
}

/// An immutable snapshot of the renderer's native text input state.
#[derive(Clone, Debug, Default)]
pub struct TextInput {
    buffer: Buffer,
    composition: Option<Composition>,
    marks: u64,
    commits: u64,
    cancellations: u64,
}

impl TextInput {
    /// Returns the current selection in UTF-16 units.
    pub fn selection(&self) -> Range<usize> {
        self.utf16_range(self.buffer.selection.clone())
    }

    /// Returns the live marked range in UTF-16 units, if any.
    pub fn marked_range(&self) -> Option<Range<usize>> {
        Some(self.utf16_range(self.composition.as_ref()?.range.clone()))
    }

    /// Returns the substring and the UTF-16 range actually used after clamping.
    pub fn text_for_range(&self, range: Range<usize>) -> (&str, Range<usize>) {
        let bytes = byte_range(&self.buffer.text, range);
        (&self.buffer.text[bytes.clone()], self.utf16_range(bytes))
    }

    pub(super) fn replace(&mut self, range: Option<Range<usize>>, text: &str) {
        self.replace_range(self.replacement_range(range), text);
        self.composition = None;
        self.commits += 1;
    }

    pub(super) fn mark(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selection: Option<Range<usize>>,
    ) {
        if text.is_empty() {
            self.cancel();
            return;
        }

        let range = self.replacement_range(range);
        let before = self
            .composition
            .take()
            .map_or_else(|| self.buffer.clone(), |composition| composition.before);
        let inserted = self.replace_range(range, text);

        if let Some(selection) = selection {
            let selection = byte_range(text, selection);
            self.buffer.selection =
                inserted.start + selection.start..inserted.start + selection.end;
        }

        self.composition = Some(Composition {
            before,
            range: inserted,
        });
        self.marks += 1;
    }

    pub(super) fn unmark(&mut self) {
        if self.composition.take().is_some() {
            self.commits += 1;
        }
    }

    pub(super) fn cancel(&mut self) {
        if let Some(composition) = self.composition.take() {
            self.buffer = composition.before;
            self.cancellations += 1;
        }
    }

    pub(super) fn backspace(&mut self) {
        if self.composition.is_some() {
            return;
        }

        let mut range = self.buffer.selection.clone();
        if range.is_empty() {
            let Some((start, _)) = self.buffer.text[..range.start].char_indices().next_back()
            else {
                return;
            };
            range.start = start;
        }

        self.buffer.text.replace_range(range.clone(), "");
        self.buffer.selection = range.start..range.start;
    }

    pub(super) fn diagnostics(&self) -> String {
        let marked = self
            .composition
            .as_ref()
            .map(|c| &self.buffer.text[c.range.clone()]);
        format!(
            "text={:?} marked={:?} marks={} commits={} text_cancel={}",
            preview(&self.buffer.text),
            marked.map(preview),
            self.marks,
            self.commits,
            self.cancellations
        )
    }

    fn replacement_range(&self, range: Option<Range<usize>>) -> Range<usize> {
        range
            .map(|range| byte_range(&self.buffer.text, range))
            .or_else(|| self.composition.as_ref().map(|c| c.range.clone()))
            .unwrap_or_else(|| self.buffer.selection.clone())
    }

    fn replace_range(&mut self, range: Range<usize>, text: &str) -> Range<usize> {
        let end = range.start + text.len();

        self.buffer.text.replace_range(range.clone(), text);
        self.buffer.selection = end..end;

        range.start..end
    }

    fn utf16_range(&self, bytes: Range<usize>) -> Range<usize> {
        let start = self.buffer.text[..bytes.start].encode_utf16().count();
        start..start + self.buffer.text[bytes].encode_utf16().count()
    }
}

fn byte_range(text: &str, range: Range<usize>) -> Range<usize> {
    let byte_at = |offset| {
        let mut units = 0;
        for (byte, ch) in text.char_indices() {
            units += ch.len_utf16();
            if units > offset {
                return byte;
            }
        }
        text.len()
    };

    byte_at(range.start)..byte_at(range.end.max(range.start))
}

fn preview(text: &str) -> &str {
    let start = text
        .char_indices()
        .rev()
        .nth(47)
        .map_or(0, |(start, _)| start);
    &text[start..]
}
