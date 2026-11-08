use std::any::Any;

use crate::gpui::SharedString;
use crate::inspection::HookMetadata;

use super::{Hooks, Kind};

pub(super) fn capture(hooks: &Hooks) -> (Vec<HookMetadata>, bool) {
    let mut truncated = hooks.slots.len() > 64;
    let metadata = hooks
        .slots
        .iter()
        .take(64)
        .enumerate()
        .map(|(index, slot)| {
            let value = preview(&*slot.value, &mut truncated);
            let kind = match slot.kind {
                Kind::State => "state",
                Kind::Ref => "ref",
                Kind::Memo => "memo",
                Kind::Effect => "effect",
                Kind::Context => "context",
                Kind::Resource => "resource",
            };

            HookMetadata {
                slot: index,
                kind: kind.into(),
                value_type: prefix(slot.type_name, 256, &mut truncated),
                value,
            }
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
