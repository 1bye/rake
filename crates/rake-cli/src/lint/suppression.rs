//! Recognize directives only in real line comments, never string literals or block comments.

use std::path::Path;

use proc_macro2::{Span, TokenStream, TokenTree};

use super::Diagnostic;
use super::catalogue::{REASON, SUPPRESSION, suppressible};

pub(super) struct Suppression {
    id: String,
    line: Option<usize>,
    unused: Option<Diagnostic>,
}

impl Suppression {
    pub(super) fn suppresses(&mut self, diagnostic: &Diagnostic) -> bool {
        if suppressible(diagnostic.rule_id)
            && self.id == diagnostic.rule_id
            && self
                .line
                .is_none_or(|line| line == diagnostic.span.start.line)
        {
            self.unused = None;

            true
        } else {
            false
        }
    }

    pub(super) fn unused(self) -> Option<Diagnostic> {
        self.unused
    }
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
    let mut offset = 0;
    let bytes = text.as_bytes();
    let mut depth = 0;
    let mut comments = Vec::new();
    let mut first_code = text.len();
    let mut literal = literals.into_iter().peekable();

    while offset < bytes.len() {
        if depth == 0 && literal.peek().is_some_and(|range| range.start == offset) {
            // Doc comments synthesize literals too, but remain valid header comments.
            if !bytes[offset..].starts_with(b"//") && !bytes[offset..].starts_with(b"/*") {
                first_code = first_code.min(offset);
            }

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
            if depth == 0 && !bytes[offset].is_ascii_whitespace() {
                first_code = first_code.min(offset);
            }

            offset += 1;
        }
    }

    directives(file, text, diagnostics, &comments, "//", first_code)
}

pub(super) fn parse_manifest(
    file: &Path,
    text: &str,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Suppression> {
    // Use the installed TOML parser's literal spans, including quoted keys,
    // multiline strings and arrays; never invent a second TOML string lexer.
    fn table_ranges(table: &toml::de::DeTable<'_>, ranges: &mut Vec<std::ops::Range<usize>>) {
        for (key, value) in table {
            ranges.push(key.span());
            value_ranges(value, ranges);
        }
    }

    fn value_ranges(
        value: &toml::Spanned<toml::de::DeValue<'_>>,
        ranges: &mut Vec<std::ops::Range<usize>>,
    ) {
        match value.get_ref() {
            toml::de::DeValue::Table(table) => table_ranges(table, ranges),
            toml::de::DeValue::Array(array) => {
                for value in array.iter() {
                    value_ranges(value, ranges);
                }
            }
            _ => ranges.push(value.span()),
        }
    }

    let Ok(table) = toml::de::DeTable::parse(text) else {
        return Vec::new();
    };

    let mut ranges = Vec::new();

    table_ranges(table.get_ref(), &mut ranges);
    ranges.sort_by_key(|range| range.start);
    let mut ranges = ranges.into_iter().peekable();
    let mut offset = 0;
    let mut comments = Vec::new();
    let mut first_code = text.len();

    while offset < text.len() {
        if ranges.peek().is_some_and(|range| range.start <= offset) {
            first_code = first_code.min(offset);
            offset = offset.max(ranges.next().expect("peeked TOML literal").end);
        } else if text.as_bytes()[offset] == b'#' {
            let end = text[offset..]
                .find('\n')
                .map_or(text.len(), |end| offset + end);

            comments.push((offset, end));
            offset = end;
        } else {
            if !text.as_bytes()[offset].is_ascii_whitespace() {
                first_code = first_code.min(offset);
            }

            offset += 1;
        }
    }

    directives(file, text, diagnostics, &comments, "#", first_code)
}

fn directives(
    file: &Path,
    text: &str,
    diagnostics: &mut Vec<Diagnostic>,
    comments: &[(usize, usize)],
    marker: &str,
    first_code: usize,
) -> Vec<Suppression> {
    let mut output = Vec::new();

    for (start, end) in comments {
        let comment = text[start + marker.len()..*end].trim();

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

        let directive = directive.trim();

        let parsed = directive
            .strip_prefix("allow-file(")
            .map(|rest| (true, rest))
            .or_else(|| directive.strip_prefix("allow(").map(|rest| (false, rest)));

        let Some((file_scope, (id, reason))) =
            parsed.and_then(|(scope, rest)| rest.split_once(')').map(|parts| (scope, parts)))
        else {
            diagnostics.push(diagnostic);
            continue;
        };

        if !suppressible(id) {
            diagnostics.push(diagnostic);
            continue;
        }

        let standalone = text[line_start..*start].trim().is_empty();

        if file_scope && (!standalone || *start > first_code) {
            diagnostic
                .message
                .push_str(": allow-file must be a standalone file-header comment");
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

        if file_scope {
            diagnostic
                .message
                .push_str(&format!(": unused file suppression for {id}"));
            output.push(Suppression {
                id: id.into(),
                line: None,
                unused: Some(diagnostic),
            });
            continue;
        }

        // A trailing comment targets its own line; a standalone directive targets
        // the next line with code (skipping blank and line-comment-only lines).
        let target = if standalone {
            text.lines()
                .enumerate()
                .skip(line)
                .find_map(|(index, content)| {
                    let content = content.trim();

                    (!content.is_empty() && !content.starts_with(marker)).then_some(index + 1)
                })
        } else {
            Some(line)
        };

        if let Some(line) = target {
            output.push(Suppression {
                id: id.into(),
                line: Some(line),
                unused: None,
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
