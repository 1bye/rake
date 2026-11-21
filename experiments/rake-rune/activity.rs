//! Shared activity data for the two native views.

use std::collections::HashSet;
use std::fmt;

pub(crate) const ROW_COUNT: usize = 1_200;
pub(crate) const WINDOW_WIDTH: f32 = 960.0;
pub(crate) const WINDOW_HEIGHT: f32 = 640.0;
pub(crate) const APP_TITLE: &str = "Rake UI Evaluation";

pub(crate) struct Activity {
    pub(crate) id: String,
    pub(crate) message: String,
}

#[derive(Debug)]
pub(crate) struct InvalidSample(usize);

impl fmt::Display for InvalidSample {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid activity sample at line {}", self.0)
    }
}

impl std::error::Error for InvalidSample {}

pub(crate) fn activity(samples: &str) -> Result<Vec<Activity>, InvalidSample> {
    let mut known = HashSet::new();
    let entries = samples
        .lines()
        .enumerate()
        .map(|(line, sample)| {
            sample
                .split_once('\t')
                .filter(|(key, text)| {
                    !key.is_empty() && !text.trim().is_empty() && known.insert(*key)
                })
                .ok_or(InvalidSample(line + 1))
        })
        .collect::<Result<Vec<_>, _>>()?;

    if entries.is_empty() {
        return Err(InvalidSample(1));
    }

    // Each event gets an identity at generation time. Sorting or filtering the rows keeps it.
    let mut events = Vec::with_capacity(ROW_COUNT);
    let mut timestamp = 1_700_000_000u64;
    for (key, text) in entries.into_iter().cycle().take(ROW_COUNT) {
        timestamp += 7 + timestamp % 11;
        events.push(Activity {
            id: format!("{key}-{timestamp}"),
            message: text.to_owned(),
        });
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_generates_unique_event_ids_and_varied_messages() {
        let events = activity(include_str!("fixtures/activity-samples.tsv")).unwrap();
        let ids: HashSet<_> = events.iter().map(|event| &event.id).collect();
        assert_eq!(events.len(), ROW_COUNT);
        assert_eq!(ids.len(), events.len());
        assert!(events.iter().any(|event| event.message.len() > 60));
        assert!(events.iter().any(|event| event.message.len() < 15));
    }

    #[test]
    fn invalid_sample_data_is_rejected() {
        for invalid in ["", "missing separator", "a\t", "a\tone\na\ttwo"] {
            assert!(activity(invalid).is_err(), "{invalid:?}");
        }
    }
}
