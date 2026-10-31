//! File processing and revision-checked workspace state, independent of the UI.

use std::cell::RefCell;
use std::sync::Arc;

/// A deliberately bounded utility, not an unbounded file editor.
pub(crate) const MAX_BYTES: usize = 128 * 1024;

#[rake::data(schema)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Statistics {
    pub bytes: usize,
    pub characters: usize,
    pub words: usize,
    pub lines: usize,
}

#[rake::data]
pub(crate) struct Processed {
    pub text: String,
    pub statistics: Statistics,
}

#[derive(Clone)]
pub(crate) struct Document {
    pub name: String,
    pub text: Arc<str>,
    pub statistics: Statistics,
}

impl Document {
    pub fn new(name: String, processed: Processed) -> Self {
        Self {
            name,
            text: processed.text.into(),
            statistics: processed.statistics,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error(transparent)]
    Host(#[from] rake::platform::Error),
    #[error(transparent)]
    Web(#[from] rake::web::Error),
    #[error("the selected file is not UTF-8 text: {0}")]
    Encoding(#[from] std::string::FromUtf8Error),
    #[error("invalid worker result: {0}")]
    Json(#[from] rake::serde_json::Error),
    #[error("the document changed before the operation completed")]
    Stale,
}

#[derive(Default)]
struct State {
    revision: u64,
    original: Option<Document>,
    current: Option<Document>,
    dirty: bool,
    download_requested: bool,
}

#[derive(Default)]
pub(crate) struct Workspace(RefCell<State>);

impl Workspace {
    pub fn current(&self) -> Option<Document> {
        self.0.borrow().current.clone()
    }

    pub fn revision(&self) -> u64 {
        self.0.borrow().revision
    }

    pub fn dirty(&self) -> bool {
        self.0.borrow().dirty
    }

    pub fn download_requested(&self) -> bool {
        self.0.borrow().download_requested
    }

    pub fn import(&self, expected: u64, document: Document) -> Result<(), Error> {
        let mut state = self.0.borrow_mut();

        if state.revision != expected {
            return Err(Error::Stale);
        }

        state.original = Some(document.clone());
        state.current = Some(document);
        state.dirty = false;
        state.download_requested = false;
        state.revision += 1;

        Ok(())
    }

    pub fn transform(&self, expected: u64, document: Document) -> Result<(), Error> {
        let mut state = self.0.borrow_mut();

        if state.revision != expected {
            return Err(Error::Stale);
        }

        state.dirty = state
            .original
            .as_ref()
            .is_some_and(|original| original.text != document.text);
        state.current = Some(document);
        state.download_requested = false;
        state.revision += 1;

        Ok(())
    }

    pub fn exported(
        &self,
        expected: u64,
        outcome: rake::platform::SaveOutcome,
    ) -> Result<(), Error> {
        let mut state = self.0.borrow_mut();

        if state.revision != expected {
            return Err(Error::Stale);
        }

        match outcome {
            rake::platform::SaveOutcome::Saved => {
                state.original = state.current.clone();
                state.dirty = false;
                state.download_requested = false;
            }
            rake::platform::SaveOutcome::DownloadRequested => state.download_requested = true,
            rake::platform::SaveOutcome::Cancelled => {}
        }

        Ok(())
    }

    pub fn confirm_download(&self) {
        let mut state = self.0.borrow_mut();

        if state.download_requested {
            state.original = state.current.clone();
            state.dirty = false;
            state.download_requested = false;
        }
    }

    pub fn discard(&self) {
        let mut state = self.0.borrow_mut();

        state.current = state.original.clone();
        state.dirty = false;
        state.download_requested = false;
        state.revision += 1;
    }
}

#[cfg(any(not(target_family = "wasm"), test))]
pub(crate) fn process(mut text: String, normalize: bool, trim: bool) -> Processed {
    if normalize {
        text = text.replace("\r\n", "\n").replace('\r', "\n");
    }

    if trim {
        text = text
            .split('\n')
            .map(|line| line.trim_end_matches([' ', '\t']))
            .collect::<Vec<_>>()
            .join("\n");
    }

    let statistics = Statistics {
        bytes: text.len(),
        characters: text.chars().count(),
        words: text.split_ascii_whitespace().count(),
        lines: if text.is_empty() {
            0
        } else {
            text.split('\n').count()
        },
    };

    Processed { text, statistics }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_processing_matches_shared_worker_fixtures() {
        let fixtures: Vec<rake::serde_json::Value> =
            rake::serde_json::from_str(include_str!("../tests/processing.json")).unwrap();

        for fixture in fixtures {
            let result = process(
                fixture["input"].as_str().unwrap().into(),
                fixture["normalize"].as_bool().unwrap(),
                fixture["trim"].as_bool().unwrap(),
            );

            assert_eq!(
                rake::serde_json::to_value(result).unwrap(),
                fixture["expected"]
            );
        }
    }

    fn document(text: &str) -> Document {
        Document::new("sample.txt".into(), process(text.into(), false, false))
    }

    #[test]
    fn processing_preserves_unicode_and_has_explicit_whitespace_rules() {
        let result = process("漢字 🙂  \r\nsecond\t\r".into(), true, true);

        assert_eq!(result.text, "漢字 🙂\nsecond\n");
        assert_eq!(result.statistics.characters, result.text.chars().count());
        assert_eq!(result.statistics.words, 3);
        assert_eq!(result.statistics.lines, 3);
        assert_eq!(process(String::new(), true, true).statistics.lines, 0);
    }

    #[test]
    fn stale_results_never_replace_a_newer_document() {
        let model = Workspace::default();

        model.import(0, document("first")).unwrap();
        let old = model.revision();

        model.import(old, document("second")).unwrap();
        assert!(matches!(
            model.transform(old, document("late")),
            Err(Error::Stale)
        ));
        assert_eq!(&*model.current().unwrap().text, "second");
    }

    #[test]
    fn browser_download_is_not_a_save_acknowledgement_and_stale_exports_are_rejected() {
        use rake::platform::SaveOutcome;
        let model = Workspace::default();

        model.import(0, document("original")).unwrap();
        model
            .transform(model.revision(), document("changed"))
            .unwrap();
        let exported = model.revision();

        model.exported(exported, SaveOutcome::Cancelled).unwrap();
        assert!(model.dirty());
        model
            .exported(exported, SaveOutcome::DownloadRequested)
            .unwrap();
        assert!(model.dirty());
        assert!(model.download_requested());
        model.confirm_download();
        assert!(!model.dirty());
        model
            .transform(model.revision(), document("another change"))
            .unwrap();
        model.discard();
        assert_eq!(&*model.current().unwrap().text, "changed");
        assert!(matches!(
            model.exported(exported, SaveOutcome::Saved),
            Err(Error::Stale)
        ));
    }
}
