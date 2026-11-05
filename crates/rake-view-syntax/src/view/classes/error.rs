//! Stable-span class diagnostics and deterministic typo suggestions.

use super::{FIXED, LENGTH_PREFIXES, PREFIXES, SCALES};

pub fn position(error: syn::Error, index: usize, class: &str) -> syn::Error {
    let n = index + 1;

    let suffix = match (n % 100, n % 10) {
        (11..=13, _) => "th",
        (_, 1) => "st",
        (_, 2) => "nd",
        (_, 3) => "rd",
        _ => "th",
    };

    syn::Error::new(
        error.span(),
        format!("{n}{suffix} class `{class}`: {error}"),
    )
}

fn distance(a: &str, b: &str) -> usize {
    let mut row: Vec<_> = (0..=b.chars().count()).collect();

    for (i, a) in a.chars().enumerate() {
        let mut diagonal = row[0];

        row[0] = i + 1;

        for (j, b) in b.chars().enumerate() {
            let previous = row[j + 1];

            row[j + 1] = (previous + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(a != b));
            diagonal = previous;
        }
    }

    *row.last().expect("distance row has an origin")
}

fn suggestion<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> String {
    let best = candidates
        .filter(|candidate| *candidate != name)
        .map(|candidate| (distance(name, candidate), candidate))
        .filter(|(distance, _)| *distance <= 2)
        .min();

    best.map(|(_, name)| format!("; did you mean `{name}`?"))
        .unwrap_or_default()
}

pub fn unknown_variant(name: &str) -> String {
    let name = format!("{name}:");

    let hint = suggestion(
        &name,
        [
            "hover:",
            "active:",
            "focus:",
            "focus-visible:",
            "in-focus:",
            "disabled:",
            "group-hover/name:",
            "group-active/name:",
        ]
        .into_iter(),
    );

    format!("unknown variant `{name}`{hint}")
}

pub fn unsupported(class: &str) -> String {
    let mut names: Vec<String> = FIXED.iter().map(|(name, _)| (*name).into()).collect();

    names.extend(["normal", "medium", "semibold", "bold"].map(|weight| format!("font-{weight}")));

    for prefix in PREFIXES {
        names.extend(SCALES.iter().map(|scale| format!("{prefix}-{scale}")));
    }

    names.extend((1..=12).map(|n| format!("grid-cols-{n}")));
    let mut hint = suggestion(class, names.iter().map(String::as_str));

    if hint.is_empty()
        && let Some((prefix, _value)) = class.rsplit_once('-')
    {
        let prefixes = PREFIXES
            .iter()
            .copied()
            .chain(LENGTH_PREFIXES.iter().map(|(prefix, _)| *prefix))
            .chain(["bg", "font", "opacity"]);

        let prefixes: Vec<_> = prefixes.collect();

        let prefix_hint = if prefixes.contains(&prefix) {
            String::new()
        } else {
            suggestion(prefix, prefixes.into_iter())
        };

        if !prefix_hint.is_empty() {
            hint = format!("; unknown prefix `{prefix}`{prefix_hint}");
        } else if ["bg-", "text-", "border-"]
            .iter()
            .any(|prefix| class.starts_with(prefix))
        {
            hint = "; colors come from your theme (`bg-surface`) or `[#rrggbb]`".into();
        } else if PREFIXES.contains(&prefix) || LENGTH_PREFIXES.iter().any(|(p, _)| *p == prefix) {
            hint = "; spacing values are whole or half steps (`p-4`, `p-1.5`) or `[value]` with px, rem or %".into();
        } else if prefix == "font" {
            hint = "; use a weight keyword or `font-[JetBrains_Mono]`".into();
        }
    }

    format!("unsupported utility class `{class}`{hint}; use style={{|s| ...}} for GPUI styling")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggestions_are_close_and_deterministic() {
        assert_eq!(distance("kitten", "sitting"), 3);
        assert_eq!(distance("", "a"), 1);
        assert!(unsupported("justify-betwen").contains("`justify-between`"));
        assert!(unknown_variant("hovr").contains("`hover:`"));
        assert!(!unsupported("completely-unknown").contains("did you mean"));
    }
}
