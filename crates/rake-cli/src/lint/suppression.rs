//! Recognize directives only in real line comments, never string literals or block comments.

use std::path::Path;

use proc_macro2::{Span, TokenStream, TokenTree};

use super::Diagnostic;
use super::catalogue::{REASON, SUPPRESSION, suppressible};

pub(super) struct Suppression {
    pub id: String,
    pub line: usize,
}

pub(super) fn parse(
    file: &Path,
    text: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Suppression> {
    let mut literals = Vec::new();

    if let Ok(tokens) = text.parse::<TokenStream>() {
        literal_ranges(tokens, &mut literals);
    } else {
        // Source parsing already emits a deny. Do not guess where comments start.
        return Vec::new();
    }

    literals.sort_by_key(|range| range.start);
    let mut output = Vec::new();
    let mut offset = 0;
    let bytes = text.as_bytes();
    let mut depth = 0;
    let mut comments = Vec::new();
    let mut literal = literals.into_iter().peekable();

    while offset < bytes.len() {
        if depth == 0 && literal.peek().is_some_and(|range| range.start == offset) {
            offset = literal.next().expect("peeked literal").end;
            continue;
        }

        if bytes[offset..].starts_with(b"/*") {
            depth += 1;
            offset += 2;
        } else if depth > 0 && bytes[offset..].starts_with(b"*/") {
            depth -= 1;
            offset += 2;
        } else if depth == 0 && bytes[offset..].starts_with(b"//") {
            let end = text[offset..]
                .find('\n')
                .map_or(text.len(), |end| offset + end);

            comments.push((offset, end));
            offset = end;
        } else {
            offset += 1;
        }
    }

    for (start, end) in &comments {
        let comment = text[start + 2..*end].trim();
        let Some(directive) = comment.strip_prefix("rake-lint:") else {
            continue;
        };
        let prefix = &text[..*start];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let line_start = prefix.rfind('\n').map_or(0, |offset| offset + 1);
        let mut diagnostic = Diagnostic::new(SUPPRESSION, file, Span::call_site());

        diagnostic.span.start.line = line;
        diagnostic.span.start.column = text[line_start..*start].chars().count() + 1;
        diagnostic.span.end.line = line;
        diagnostic.span.end.column = text[line_start..*end].chars().count() + 1;

        let Some((id, reason)) = directive
            .trim()
            .strip_prefix("allow(")
            .and_then(|rest| rest.split_once(')'))
        else {
            diagnostics.push(diagnostic);
            continue;
        };

        if !suppressible(id) {
            diagnostics.push(diagnostic);
            continue;
        }

        if reason.trim().is_empty() {
            let rule = super::catalogue::rule(REASON).expect("reason rule exists");

            diagnostic.rule_id = rule.id;
            diagnostic.level = rule.level;
            diagnostic.message = rule.message.into();
            diagnostic.explanation = rule.explanation;
            diagnostic.fix = rule.fix;
            diagnostics.push(diagnostic);
            continue;
        }

        // A trailing comment targets its own line; a standalone directive targets
        // the next line with code (skipping blank and line-comment-only lines).
        let target = if text[line_start..*start].trim().is_empty() {
            text.lines()
                .enumerate()
                .skip(line)
                .find_map(|(index, content)| {
                    let content = content.trim();

                    (!content.is_empty() && !content.starts_with("//")).then_some(index + 1)
                })
        } else {
            Some(line)
        };

        if let Some(line) = target {
            output.push(Suppression {
                id: id.into(),
                line,
            });
        }
    }

    output
}

fn literal_ranges(tokens: TokenStream, ranges: &mut Vec<std::ops::Range<usize>>) {
    for token in tokens {
        match token {
            TokenTree::Literal(literal) => ranges.push(literal.span().byte_range()),
            TokenTree::Group(group) => literal_ranges(group.stream(), ranges),
            _ => {}
        }
    }
}
