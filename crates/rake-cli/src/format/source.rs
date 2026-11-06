//! Token and source-gap operations: generated output always retains original lexemes.

use std::ops::Range;

use proc_macro2::{Delimiter, LineColumn, TokenStream, TokenTree};

pub(super) fn offset(source: &str, point: LineColumn) -> usize {
    let line = source
        .split_inclusive('\n')
        .take(point.line.saturating_sub(1))
        .map(str::len)
        .sum::<usize>();
    line + source[line..]
        .chars()
        .take(point.column)
        .map(char::len_utf8)
        .sum::<usize>()
}

pub(super) fn span_range(
    source: &str,
    start: proc_macro2::Span,
    end: proc_macro2::Span,
) -> Range<usize> {
    offset(source, start.start())..offset(source, end.end())
}

#[derive(Debug)]
pub(super) struct Atom<'a> {
    pub range: Range<usize>,
    pub text: &'a str,
}

/// Include comments as indivisible atoms, even though proc-macro tokens omit them.
pub(super) fn atoms(source: &str) -> Option<Vec<Atom<'_>>> {
    fn walk(source: &str, tokens: TokenStream, ranges: &mut Vec<Range<usize>>) {
        for token in tokens {
            if let TokenTree::Group(group) = token {
                if group.delimiter() != Delimiter::None {
                    ranges.push(span_range(source, group.span_open(), group.span_open()));
                }
                walk(source, group.stream(), ranges);
                if group.delimiter() != Delimiter::None {
                    ranges.push(span_range(source, group.span_close(), group.span_close()));
                }
            } else {
                ranges.push(span_range(source, token.span(), token.span()));
            }
        }
    }

    let mut ranges = Vec::new();
    walk(source, source.parse().ok()?, &mut ranges);
    ranges.sort_by_key(|range| (range.start, std::cmp::Reverse(range.end)));
    let mut result = Vec::new();
    let mut end = 0;
    for range in ranges {
        // Doc comments synthesize several tokens with the same source range.
        if range.start < end || range.is_empty() {
            continue;
        }
        comments(source, end..range.start, &mut result)?;
        result.push(Atom {
            text: source.get(range.clone())?,
            range: range.clone(),
        });
        end = range.end;
    }
    comments(source, end..source.len(), &mut result)?;
    Some(result)
}

fn comments<'a>(source: &'a str, gap: Range<usize>, result: &mut Vec<Atom<'a>>) -> Option<()> {
    let mut position = gap.start;
    while position < gap.end {
        let rest = source.get(position..gap.end)?;
        if rest.starts_with("//") {
            let end = position + rest.find('\n').unwrap_or(rest.len());
            result.push(Atom {
                range: position..end,
                text: &source[position..end],
            });
            position = end;
        } else if rest.starts_with("/*") {
            let start = position;
            position += 2;
            let mut depth = 1;
            while depth > 0 && position < gap.end {
                if source[position..].starts_with("/*") {
                    depth += 1;
                    position += 2;
                } else if source[position..].starts_with("*/") {
                    depth -= 1;
                    position += 2;
                } else {
                    position += source[position..].chars().next()?.len_utf8();
                }
            }
            if depth != 0 {
                return None;
            }
            result.push(Atom {
                range: start..position,
                text: &source[start..position],
            });
        } else {
            let character = rest.chars().next()?;
            if !character.is_whitespace() {
                return None;
            }
            position += character.len_utf8();
        }
    }
    Some(())
}

pub(super) fn equivalent(before: &str, after: &str) -> bool {
    match (atoms(before), atoms(after)) {
        (Some(before), Some(after)) => before
            .iter()
            .map(|atom| atom.text)
            .eq(after.iter().map(|atom| atom.text)),
        _ => false,
    }
}

/// Project rustfmt's whitespace onto the original atoms. Added commas and removed
/// redundant delimiters are never copied into markup.
pub(super) fn project(original: &str, formatted: &str) -> String {
    let (Some(before), Some(after)) = (atoms(original), atoms(formatted)) else {
        return original.into();
    };
    let mut output = String::new();
    let mut cursor = 0;
    let mut previous_original = 0;
    for atom in &before {
        let found = after[cursor..]
            .iter()
            .position(|candidate| candidate.text == atom.text);
        if let Some(found) = found {
            let index = cursor + found;
            let gap_start = if index == 0 {
                0
            } else {
                after[index - 1].range.end
            };
            let gap = &formatted[gap_start..after[index].range.start];
            if !output.is_empty() {
                output.push_str(gap);
            }
            cursor = index + 1;
        } else {
            output.push_str(&original[previous_original..atom.range.start]);
        }
        output.push_str(atom.text);
        previous_original = atom.range.end;
    }
    if equivalent(original, &output) {
        output
    } else {
        original.into()
    }
}

pub(super) fn compact(source: &str, tight: bool) -> String {
    let Some(atoms) = atoms(source) else {
        return source.into();
    };
    let mut result = String::new();
    let mut previous: Option<&Atom<'_>> = None;
    for atom in &atoms {
        if let Some(left) = previous {
            let gap = &source[left.range.end..atom.range.start];
            if left.text.starts_with("//") {
                result.push('\n');
            } else if !gap.is_empty()
                && (!tight
                    || left.text.starts_with("/*")
                    || atom.text.starts_with("/*")
                    || atom.text.starts_with("//"))
            {
                result.push(' ');
            }
        }
        result.push_str(atom.text);
        previous = Some(atom);
    }
    if previous.is_some_and(|atom| atom.text.starts_with("//")) {
        result.push('\n');
    }
    result
}

/// Rustfmt owns surrounding Rust; markup bodies remain opaque until our parser
/// formats them, including bodies that are intentionally invalid.
pub(super) fn preserve_views(before: &str, after: &str) -> String {
    fn bodies(source: &str) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        if let Ok(tokens) = source.parse() {
            crate::view_source::nested_views(tokens, &mut |invocation| {
                ranges.push(
                    span_range(source, invocation.open, invocation.open).end
                        ..span_range(source, invocation.close, invocation.close).start,
                );
            });
        }
        ranges
    }
    let before_ranges = bodies(before);
    let after_ranges = bodies(after);
    if before_ranges.len() != after_ranges.len() {
        return after.into();
    }
    let mut output = after.to_owned();
    for (original, formatted) in before_ranges.into_iter().zip(after_ranges).rev() {
        output.replace_range(formatted, &before[original]);
    }
    output
}
