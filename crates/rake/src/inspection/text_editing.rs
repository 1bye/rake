//! Bounded text overrides; None means the current application-owned content.

use super::{InspectionError, Store, TextEdit};

const MAX_TEXT_BYTES: usize = 16 * 1024;
const MAX_HISTORY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Default)]
pub(super) struct History {
    pub current: Option<String>,
    undo: Vec<Option<String>>,
}

impl History {
    fn bytes(&self) -> usize {
        self.current
            .iter()
            .chain(self.undo.iter().flatten())
            .map(String::len)
            .sum()
    }
}

impl Store {
    pub(crate) fn edit_text(
        &self,
        id: &str,
        generation: &str,
        edit: &TextEdit,
    ) -> Result<(), InspectionError> {
        self.available()?;
        if let TextEdit::Set { text } = edit
            && text.len() > MAX_TEXT_BYTES
        {
            return Err(rejected("text exceeds 16 KiB"));
        }
        let mut state = self.0.borrow_mut();
        let element = state
            .windows
            .values()
            .find(|host| host.snapshot.capture == generation)
            .ok_or(InspectionError::StaleGeneration)?
            .snapshot
            .elements
            .iter()
            .find(|element| element.id == id)
            .ok_or(InspectionError::NotRetained)?;
        let stable = element
            .stable_id
            .as_ref()
            .filter(|_| element.text_editable && element.text.is_some())
            .ok_or_else(|| rejected("boundary has no editable stable text"))?
            .clone();
        if state.text_edits.len() >= 64 && !state.text_edits.contains_key(&stable) {
            return Err(rejected("64 edited text boundaries already retained"));
        }
        let mut next = state.text_edits.get(&stable).cloned().unwrap_or_default();
        match edit {
            TextEdit::Undo {} => {
                next.current = next
                    .undo
                    .pop()
                    .ok_or_else(|| rejected("no previous text edit"))?
            }
            _ => {
                let value = match edit {
                    TextEdit::Set { text } => Some(text.clone()),
                    _ => None,
                };
                if next.current == value {
                    return Ok(());
                }
                if next.undo.len() == 32 {
                    next.undo.remove(0);
                }
                next.undo.push(std::mem::replace(&mut next.current, value));
            }
        }
        let bytes: usize = state
            .text_edits
            .iter()
            .filter(|(key, _)| *key != &stable)
            .map(|(_, history)| history.bytes())
            .sum();
        if bytes + next.bytes() > MAX_HISTORY_BYTES {
            return Err(rejected("text history exceeds 4 MiB"));
        }
        if next.current.is_none() && next.undo.is_empty() {
            state.text_edits.remove(&stable);
        } else {
            state.text_edits.insert(stable, next);
        }
        state.changed.notify(usize::MAX);
        Ok(())
    }
}

fn rejected(message: &str) -> InspectionError {
    InspectionError::TextEdit(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::WindowId;

    fn host() -> (Store, String) {
        let store = Store::default();
        let mut snapshot = store.begin(0);
        snapshot.elements.push(
            serde_json::from_value(serde_json::json!({
                "id":"text", "kind":"Text", "text":"Application", "text_editable":true,
                "stable_id":"stable-text", "metadata_truncated":false
            }))
            .unwrap(),
        );
        let generation = snapshot.capture.clone();
        store.publish(WindowId(1), snapshot);
        (store, generation)
    }

    fn value(store: &Store) -> Option<String> {
        store
            .0
            .borrow()
            .text_edits
            .get("stable-text")
            .and_then(|h| h.current.clone())
    }

    #[test]
    fn empty_unicode_reset_undo_and_no_op_edits_are_distinct() {
        let (store, generation) = host();
        for text in ["café 你好 🙂", ""] {
            store
                .edit_text("text", &generation, &TextEdit::Set { text: text.into() })
                .unwrap();
            assert_eq!(value(&store).as_deref(), Some(text));
        }
        store
            .edit_text("text", &generation, &TextEdit::Set { text: "".into() })
            .unwrap();
        store
            .edit_text("text", &generation, &TextEdit::Reset {})
            .unwrap();
        assert_eq!(value(&store), None);
        store
            .edit_text("text", &generation, &TextEdit::Undo {})
            .unwrap();
        assert_eq!(value(&store).as_deref(), Some(""));
        store
            .edit_text("text", &generation, &TextEdit::Undo {})
            .unwrap();
        assert_eq!(value(&store).as_deref(), Some("café 你好 🙂"));
        store
            .edit_text("text", &generation, &TextEdit::Undo {})
            .unwrap();
        assert_eq!(value(&store), None);
        assert!(
            store
                .edit_text("text", &generation, &TextEdit::Undo {})
                .is_err()
        );
        assert!(store.0.borrow().text_edits.is_empty());
        assert_eq!(
            store.element("text").unwrap().text.as_deref(),
            Some("Application"),
            "captures reflect draws, not pending edits"
        );
    }

    #[test]
    fn edits_validate_identity_byte_limits_and_expire_on_removal() {
        let (store, generation) = host();
        let set = |text: String| TextEdit::Set { text };
        assert!(store.edit_text("text", "stale", &set("x".into())).is_err());
        assert!(
            store
                .edit_text("missing", &generation, &set("x".into()))
                .is_err()
        );
        assert!(
            store
                .edit_text("text", &generation, &set("🙂".repeat(4097)))
                .is_err()
        );
        store
            .edit_text("text", &generation, &set("🙂".repeat(4096)))
            .unwrap();
        let current = value(&store);
        assert!(
            store
                .edit_text("text", &generation, &set("x".repeat(MAX_TEXT_BYTES + 1)))
                .is_err()
        );
        assert_eq!(value(&store), current);
        let snapshot = store.begin(0);
        store.publish(WindowId(1), snapshot);
        assert!(store.0.borrow().text_edits.is_empty());
    }

    #[test]
    fn history_is_bounded_and_budget_rejections_are_atomic() {
        let (store, generation) = host();
        for i in 0..40 {
            store
                .edit_text(
                    "text",
                    &generation,
                    &TextEdit::Set {
                        text: i.to_string(),
                    },
                )
                .unwrap();
        }
        for _ in 0..32 {
            store
                .edit_text("text", &generation, &TextEdit::Undo {})
                .unwrap();
        }
        assert_eq!(value(&store).as_deref(), Some("7"));
        assert!(
            store
                .edit_text("text", &generation, &TextEdit::Undo {})
                .is_err()
        );
        let mut snapshot = store.begin(0);
        let template = store.element("text").unwrap();
        snapshot.elements.push(template.clone());
        for i in 0..8 {
            let mut element = template.clone();
            element.id = format!("other-{i}");
            element.stable_id = Some(element.id.clone());
            snapshot.elements.push(element);
        }
        let generation = snapshot.capture.clone();
        store.publish(WindowId(1), snapshot);
        let mut exhausted = false;
        'fill: for i in 0..8 {
            for step in 0..33 {
                let edit = TextEdit::Set {
                    text: format!("{i}:{step}:{}", "x".repeat(MAX_TEXT_BYTES - 10)),
                };
                if store
                    .edit_text(&format!("other-{i}"), &generation, &edit)
                    .is_err()
                {
                    exhausted = true;
                    break 'fill;
                }
            }
        }
        assert!(exhausted);
        let before = value(&store);
        assert!(
            store
                .edit_text(
                    "text",
                    &generation,
                    &TextEdit::Set {
                        text: "x".repeat(MAX_TEXT_BYTES)
                    }
                )
                .is_err()
        );
        assert_eq!(value(&store), before);
    }
}
