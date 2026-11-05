//! Derive a contiguous browser edit relative to the original selection.

use std::ops::Range;

/// The browser already replaced the selection, even where the old and new
/// text match. Matching context must stay outside that selection.
pub(crate) fn replacement(
    old: &[u16],
    new: &[u16],
    selection: Range<usize>,
    post_edit_caret: Option<usize>,
) -> (usize, usize, String) {
    let trailing = old.len().saturating_sub(selection.end);
    let mut suffix = post_edit_caret
        .and_then(|caret| new.len().checked_sub(caret))
        .filter(|&length| {
            length <= trailing && old[old.len() - length..] == new[new.len() - length..]
        })
        .unwrap_or_else(|| {
            old.iter()
                .rev()
                .zip(new.iter().rev())
                .take(trailing)
                .take_while(|(a, b)| a == b)
                .count()
        });

    while splits_scalar(old, old.len() - suffix) || splits_scalar(new, new.len() - suffix) {
        suffix -= 1;
    }

    let mut prefix = old
        .iter()
        .zip(new)
        .take(
            selection
                .start
                .min(old.len() - suffix)
                .min(new.len() - suffix),
        )
        .take_while(|(a, b)| a == b)
        .count();

    while splits_scalar(old, prefix) || splits_scalar(new, prefix) {
        prefix -= 1;
    }

    (
        selection.start.saturating_sub(prefix),
        (old.len() - suffix).saturating_sub(selection.end),
        String::from_utf16_lossy(&new[prefix..new.len() - suffix]),
    )
}

fn splits_scalar(text: &[u16], offset: usize) -> bool {
    matches!(
        (
            offset.checked_sub(1).and_then(|i| text.get(i)),
            text.get(offset)
        ),
        (Some(0xd800..=0xdbff), Some(0xdc00..=0xdfff))
    )
}

#[cfg(test)]
mod tests {
    use super::replacement;
    use std::ops::Range;

    fn apply(old: &str, new: &str, selected: Range<usize>, caret: Option<usize>) {
        let mut old: Vec<_> = old.encode_utf16().collect();
        let (before, after, inserted) = replacement(
            &old,
            &new.encode_utf16().collect::<Vec<_>>(),
            selected.clone(),
            caret,
        );
        old.splice(
            selected.start - before..selected.end + after,
            inserted.encode_utf16(),
        );

        assert_eq!(String::from_utf16(&old).unwrap(), new);
    }

    #[test]
    fn replacement_keeps_shared_selected_prefix_and_suffix() {
        apply("A little space to think", "AX title", 0..23, Some(8));
        apply(
            "before same end after",
            "before other end after",
            7..15,
            None,
        );
        apply("😀", "😁", 0..2, Some(2));
    }

    #[test]
    fn replacement_expands_shared_surrogates_outside_selection() {
        apply("a😀b", "a😁b", 3..3, Some(3));
        apply("a😀b", "a🈀b", 1..1, None);
    }

    #[test]
    fn insertion_and_delete_keep_the_intended_context() {
        apply("pact", "pactor pact", 0..0, Some(7));
        apply("word world", "word", 10..10, Some(4));
    }
}
