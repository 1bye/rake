use super::signal::{Key, Signal};
use crate::gpui::{App, SharedString};
use crate::inspection::SignalMetadata;
use std::any::{Any, type_name};

pub(super) struct Entry {
    name: &'static str,
    kind: &'static str,
    value_type: &'static str,
    key: Key,
    preview: fn(Key, &App, &mut bool) -> Option<serde_json::Value>,
}

impl Entry {
    pub fn new<T: 'static>(name: &'static str, kind: &'static str, signal: Signal<T>) -> Self {
        Self {
            name,
            kind,
            value_type: type_name::<T>(),
            key: signal.key,
            preview: |key, cx, truncated| {
                let signal = Signal::<T>::from_key(key);
                preview(signal.resolve(cx)?.read(cx), truncated)
            },
        }
    }
}

pub(super) fn capture(entries: &[Entry], cx: &App) -> (Vec<SignalMetadata>, bool) {
    let mut truncated = entries.len() > 64;
    let metadata = entries
        .iter()
        .take(64)
        .map(|entry| SignalMetadata {
            name: prefix(entry.name, 128, &mut truncated),
            kind: entry.kind.into(),
            value_type: prefix(entry.value_type, 256, &mut truncated),
            value: (entry.preview)(entry.key, cx, &mut truncated),
        })
        .collect();
    (metadata, truncated)
}

fn preview(value: &dyn Any, truncated: &mut bool) -> Option<serde_json::Value> {
    macro_rules! scalar {
        ($($type:ty),*) => {
            $(if let Some(value) = value.downcast_ref::<$type>() {
                return Some(serde_json::json!(value));
            })*
        };
    }
    scalar!(
        bool, u8, u16, u32, u64, usize, i8, i16, i32, i64, isize, f32, f64
    );
    let text = value
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| {
            value
                .downcast_ref::<SharedString>()
                .map(|value| value.as_ref())
        })
        .or_else(|| value.downcast_ref::<&'static str>().copied())?;

    Some(serde_json::Value::String(prefix(text, 256, truncated)))
}

fn prefix(value: &str, limit: usize, truncated: &mut bool) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }

    *truncated = true;
    let end = value.floor_char_boundary(limit);

    value[..end].to_owned()
}
