//! Render the shared parser's source layout without recreating tokens.

use std::ops::Range;

use rake_view_syntax::{Layout, LayoutKind, View};

use super::rustfmt::Rustfmt;
use super::source::{atoms, compact, equivalent, span_range};
use super::{Problem, Result};
use crate::view_source;

pub(super) fn format_source(source: &str, rustfmt: &mut Rustfmt) -> Result<(String, Vec<Problem>)> {
    let mut current = source.to_owned();
    for _ in 0..8 {
        let (formatted, problems) = format_once(&current, rustfmt)?;
        if formatted == current {
            return Ok((formatted, problems));
        }
        current = formatted;
    }
    Ok((
        source.into(),
        vec![Problem {
            line: 1,
            column: 1,
            message: "formatter did not converge; source left unchanged".into(),
        }],
    ))
}

fn format_once(source: &str, rustfmt: &mut Rustfmt) -> Result<(String, Vec<Problem>)> {
    let invocations = match view_source::invocations(source) {
        Ok(invocations) => invocations,
        Err(error) => return Ok((source.into(), vec![Problem::parse(error)])),
    };
    let mut replacements = Vec::new();
    let mut problems = Vec::new();
    for invocation in invocations {
        nested_problems(invocation.tokens.clone(), &mut problems);
        let start = span_range(source, invocation.open, invocation.open).end;
        let end = span_range(source, invocation.close, invocation.close).start;
        let view = match syn::parse2::<View>(invocation.tokens).and_then(|view| {
            view.layout()?;
            Ok(view)
        }) {
            Ok(view) => view,
            Err(error) => {
                problems.push(Problem::parse(error));
                continue;
            }
        };
        let line_start = source[..start]
            .rfind('\n')
            .map_or(0, |position| position + 1);
        let leading: String = source[line_start..start]
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect();
        let base = rustfmt.columns(&leading);
        let indent = base + rustfmt.tab_spaces;
        let mut renderer = Renderer { source, rustfmt };
        let layout = view.layout()?;
        let root = renderer.render(layout, indent)?;
        let range = renderer.range(layout);
        let before = renderer.gap(
            start..range.start,
            &format!("\n{}", renderer.rustfmt.indent(indent)),
        );
        let after = renderer.gap(
            range.end..end,
            &format!("\n{}", renderer.rustfmt.indent(base)),
        );
        let result = format!("{before}{root}{after}");
        if !equivalent(&source[start..end], &result) {
            problems.push(Problem {
                line: invocation.open.start().line,
                column: invocation.open.start().column + 1,
                message: "formatter preservation check failed; invocation left unchanged".into(),
            });
            continue;
        }
        // Unquoted markup text adopts source gaps, so token equivalence alone
        // cannot establish that its lowered string is unchanged.
        let reparsed = syn::parse_str::<View>(&result).and_then(|view| {
            view.layout()?;
            Ok(view)
        });
        if let Ok(reparsed) = reparsed {
            if text_values(&view) != text_values(&reparsed) {
                problems.push(Problem {
                    line: invocation.open.start().line,
                    column: invocation.open.start().column + 1,
                    message: "unquoted text preservation check failed; invocation left unchanged"
                        .into(),
                });
                continue;
            }
        } else {
            problems.push(Problem {
                line: invocation.open.start().line,
                column: invocation.open.start().column + 1,
                message: "formatted invocation did not parse; invocation left unchanged".into(),
            });
            continue;
        }
        replacements.push((start..end, result));
    }
    let mut output = source.to_owned();
    for (range, replacement) in replacements.into_iter().rev() {
        output.replace_range(range, &replacement);
    }
    Ok((output, problems))
}

struct Renderer<'a> {
    source: &'a str,
    rustfmt: &'a mut Rustfmt,
}

impl Renderer<'_> {
    fn range(&self, layout: &Layout) -> Range<usize> {
        span_range(self.source, layout.start, layout.end)
    }

    fn raw(&self, layout: &Layout) -> &str {
        &self.source[self.range(layout)]
    }

    fn gap(&self, range: Range<usize>, desired: &str) -> String {
        self.layout_gap(range, desired, false)
    }

    fn sibling_gap(&self, range: Range<usize>, desired: &str) -> String {
        self.layout_gap(range, desired, true)
    }

    fn has_blank_line(&self, range: Range<usize>) -> bool {
        let source = &self.source[range];
        let Some(parts) = atoms(source) else {
            return false;
        };
        let mut previous = 0;
        for part in parts {
            if source[previous..part.range.start].matches('\n').count() > 1 {
                return true;
            }
            previous = part.range.end;
        }
        source[previous..].matches('\n').count() > 1
    }

    fn layout_gap(&self, range: Range<usize>, desired: &str, preserve_blank: bool) -> String {
        let source = &self.source[range];
        let whitespace = |gap: &str| {
            if preserve_blank && desired.contains('\n') && gap.matches('\n').count() > 1 {
                format!("\n{desired}")
            } else {
                desired.to_owned()
            }
        };
        if source.trim().is_empty() {
            return whitespace(source);
        }
        let Some(parts) = atoms(source) else {
            return source.into();
        };
        let mut result = String::new();
        let mut previous = 0;
        for part in &parts {
            let before = &source[previous..part.range.start];
            if before.contains('\n') {
                if preserve_blank && before.matches('\n').count() > 1 {
                    result.push('\n');
                }
                result.push('\n');
                result.push_str(desired.rsplit_once('\n').map_or("", |(_, indent)| indent));
            } else {
                result.push(' ');
            }
            result.push_str(part.text);
            previous = part.range.end;
        }
        let trailing_line_comment = parts.last().is_some_and(|part| part.text.starts_with("//"));
        if trailing_line_comment && !desired.contains('\n') {
            result.push('\n');
        } else {
            result.push_str(&whitespace(&source[previous..]));
        }
        result
    }

    fn render(&mut self, layout: &Layout, indent: usize) -> Result<String> {
        match layout.kind {
            LayoutKind::Element => self.element(layout, indent),
            LayoutKind::Opening => self.opening(layout, indent),
            LayoutKind::Attribute => self.attribute(layout, indent),
            LayoutKind::Block => self.block(layout, indent),
            LayoutKind::Flow => self.flow(layout, indent),
            LayoutKind::Rust => self.rust(layout, indent),
            LayoutKind::Header => {
                let source = self.raw(layout).to_owned();
                self.rustfmt.rust(&source, indent, true)
            }
            LayoutKind::Closing => Ok(compact(self.raw(layout), true)),
            LayoutKind::Literal | LayoutKind::Text => Ok(self.raw(layout).into()),
        }
    }

    fn element(&mut self, layout: &Layout, indent: usize) -> Result<String> {
        let opening = &layout.children[0];
        let mut output = self.opening(opening, indent)?;
        if layout.children.len() == 1 {
            return Ok(output);
        }
        let closing = layout.children.last().expect("nonempty element layout");
        let children = &layout.children[1..layout.children.len() - 1];
        let close = self.render(closing, indent)?;
        let content_start = self.range(opening).end;
        let content_end = self.range(closing).start;
        let originally_inline = children.first().is_some_and(|first| {
            opening.end.end().line == first.start.start().line
                && children
                    .last()
                    .is_some_and(|last| first.start.start().line == last.end.end().line)
        });
        if !output.contains('\n') && (children.len() <= 1 || originally_inline) {
            let mut inline = String::new();
            let mut previous = content_start;
            let mut eligible = true;
            for child in children {
                let range = self.range(child);
                let gap = &self.source[previous..range.start];
                if !gap.trim().is_empty()
                    || self.has_blank_line(previous..range.start)
                    || child.kind == LayoutKind::Element
                        && !children.iter().any(|child| child.kind == LayoutKind::Text)
                    || !matches!(
                        child.kind,
                        LayoutKind::Text
                            | LayoutKind::Literal
                            | LayoutKind::Rust
                            | LayoutKind::Element
                    )
                {
                    eligible = false;
                    break;
                }
                // Keep same-line gaps: unquoted text adopts their whitespace.
                if !gap.contains('\n') {
                    inline.push_str(gap);
                }
                inline.push_str(&self.render(child, indent)?);
                previous = range.end;
            }
            let trailing = &self.source[previous..content_end];
            eligible &= trailing.trim().is_empty() && !self.has_blank_line(previous..content_end);
            if !trailing.contains('\n') {
                inline.push_str(trailing);
            }
            if eligible
                && !inline.contains('\n')
                && indent
                    + self.rustfmt.columns(&output)
                    + self.rustfmt.columns(&inline)
                    + self.rustfmt.columns(&close)
                    <= self.rustfmt.width
            {
                output.push_str(&inline);
                output.push_str(&close);
                return Ok(output);
            }
        }
        let nested = indent + self.rustfmt.tab_spaces;
        let mut previous = self.range(opening).end;
        let mut previous_text = false;
        for (index, child) in children.iter().enumerate() {
            let range = self.range(child);
            let source_gap = &self.source[previous..range.start];
            let gap = if (previous_text || child.kind == LayoutKind::Text && index > 0)
                && !source_gap.contains('\n')
            {
                source_gap.to_owned()
            } else if index > 0 {
                self.sibling_gap(
                    previous..range.start,
                    &format!("\n{}", self.rustfmt.indent(nested)),
                )
            } else {
                self.gap(
                    previous..range.start,
                    &format!("\n{}", self.rustfmt.indent(nested)),
                )
            };
            output.push_str(&gap);
            output.push_str(&self.render(child, nested)?);
            previous = range.end;
            previous_text = child.kind == LayoutKind::Text;
        }
        output.push_str(&self.gap(
            previous..self.range(closing).start,
            &format!("\n{}", self.rustfmt.indent(indent)),
        ));
        output.push_str(&close);
        Ok(output)
    }

    fn opening(&mut self, layout: &Layout, indent: usize) -> Result<String> {
        let parts = &layout.children;
        let name = compact(self.raw(&parts[0]), true);
        let end = parts.last().expect("opening layout has delimiters");
        let close = compact(self.raw(end), true);
        let attributes = &parts[1..parts.len() - 1];
        let nested = indent + self.rustfmt.tab_spaces;
        let mut values = Vec::new();
        for attribute in attributes {
            values.push(self.attribute(attribute, nested)?);
        }
        let short = if values.is_empty() {
            format!("{name}{close}")
        } else {
            let separator = if close == "/>" { " " } else { "" };
            format!("{name} {}{separator}{close}", values.join(" "))
        };
        let comments = parts.windows(2).any(|pair| {
            !self.source[self.range(&pair[0]).end..self.range(&pair[1]).start]
                .trim()
                .is_empty()
        });
        let blank_lines = attributes
            .windows(2)
            .any(|pair| self.has_blank_line(self.range(&pair[0]).end..self.range(&pair[1]).start));
        let split = blank_lines
            || attributes.len() > 3
            || comments
            || short.contains('\n')
            || indent + self.rustfmt.columns(&short) > self.rustfmt.width;
        let mut output = name;
        let mut previous = self.range(&parts[0]).end;
        for (index, (attribute, value)) in attributes.iter().zip(values).enumerate() {
            let desired = if split {
                format!("\n{}", self.rustfmt.indent(nested))
            } else {
                " ".into()
            };
            output.push_str(&self.layout_gap(
                previous..self.range(attribute).start,
                &desired,
                index > 0,
            ));
            output.push_str(&value);
            previous = self.range(attribute).end;
        }
        let desired = if split {
            format!("\n{}", self.rustfmt.indent(indent))
        } else if close == "/>" && !attributes.is_empty() {
            " ".into()
        } else {
            String::new()
        };
        output.push_str(&self.gap(previous..self.range(end).start, &desired));
        output.push_str(&close);
        Ok(output)
    }

    fn attribute(&mut self, layout: &Layout, indent: usize) -> Result<String> {
        let source = self.raw(layout).to_owned();
        let tokens: proc_macro2::TokenStream = source.parse()?;
        let group = tokens.into_iter().find_map(|token| match token {
            proc_macro2::TokenTree::Group(group)
                if group.delimiter() == proc_macro2::Delimiter::Brace =>
            {
                Some(group)
            }
            _ => None,
        });
        if let Some(group) = group {
            let range = span_range(&source, group.span_open(), group.span_close());
            if let Some(element) = layout.children.first() {
                let attribute = self.range(layout);
                let child = self.range(element);
                let nested = indent + self.rustfmt.tab_spaces;
                let before = self.gap(
                    attribute.start + range.start + 1..child.start,
                    &format!("\n{}", self.rustfmt.indent(nested)),
                );
                let after = self.gap(
                    child.end..attribute.start + range.end - 1,
                    &format!("\n{}", self.rustfmt.indent(indent)),
                );
                return Ok(format!(
                    "{}{{{before}{}{after}}}{}",
                    compact(&source[..range.start], true),
                    self.render(element, nested)?,
                    compact(&source[range.end..], true)
                ));
            }
            let rust = self.rust_text(&source[range.clone()], indent)?;
            Ok(format!(
                "{}{}{}",
                compact(&source[..range.start], true),
                rust,
                compact(&source[range.end..], true)
            ))
        } else {
            Ok(compact(&source, true))
        }
    }

    fn rust(&mut self, layout: &Layout, indent: usize) -> Result<String> {
        self.rust_text(&self.source[self.range(layout)], indent)
    }

    fn rust_text(&mut self, source: &str, indent: usize) -> Result<String> {
        let formatted = self.rustfmt.rust(source, indent, false)?;
        // Nested macros are discovered again after rustfmt establishes Rust indentation.
        // A fragment is a valid expression when wrapped as a function initializer.
        let prefix = format!(
            "{}fn __rake_nested() {{ let _ = ",
            self.rustfmt.indent(indent)
        );
        let wrapped = format!("{prefix}{formatted}; }}");
        let (nested, _) = format_source(&wrapped, self.rustfmt)?;
        Ok(nested[prefix.len()..nested.len() - 3].to_owned())
    }

    fn block(&mut self, layout: &Layout, indent: usize) -> Result<String> {
        let range = self.range(layout);
        let open = range.start + 1;
        let close = range.end - 1;
        let nested = indent + self.rustfmt.tab_spaces;
        let mut output = String::from("{");
        let mut previous = open;
        let mut previous_text = false;
        for (index, child) in layout.children.iter().enumerate() {
            let child_range = self.range(child);
            let source_gap = &self.source[previous..child_range.start];
            let gap = if (previous_text || child.kind == LayoutKind::Text && index > 0)
                && !source_gap.contains('\n')
            {
                source_gap.to_owned()
            } else {
                self.layout_gap(
                    previous..child_range.start,
                    &format!("\n{}", self.rustfmt.indent(nested)),
                    index > 0,
                )
            };
            output.push_str(&gap);
            output.push_str(&self.render(child, nested)?);
            previous = child_range.end;
            previous_text = child.kind == LayoutKind::Text;
        }
        let desired = if layout.children.is_empty() && self.source[open..close].trim().is_empty() {
            String::new()
        } else {
            format!("\n{}", self.rustfmt.indent(indent))
        };
        output.push_str(&self.gap(previous..close, &desired));
        output.push('}');
        Ok(output)
    }

    fn flow(&mut self, layout: &Layout, indent: usize) -> Result<String> {
        let mut output = String::new();
        let mut previous = self.range(layout).start;
        for child in &layout.children {
            let range = self.range(child);
            let desired = if output.is_empty() || self.raw(child) == "," {
                ""
            } else {
                " "
            };
            output.push_str(&self.gap(previous..range.start, desired));
            output.push_str(&self.render(child, indent)?);
            previous = range.end;
        }
        Ok(output)
    }
}

fn nested_problems(tokens: proc_macro2::TokenStream, problems: &mut Vec<Problem>) {
    view_source::nested_views(tokens, &mut |invocation| match syn::parse2::<View>(
        invocation.tokens.clone(),
    )
    .and_then(|view| view.layout().map(|_| ()))
    {
        Ok(()) => nested_problems(invocation.tokens, problems),
        Err(error) => problems.push(Problem::parse(error)),
    });
}

fn text_values(view: &View) -> Vec<String> {
    use rake_view_syntax::{AttributeValue, Element, Node};
    fn element(value: &Element, output: &mut Vec<String>) {
        for attribute in &value.attributes {
            if let AttributeValue::Markup(child) = &attribute.value {
                element(child, output);
            }
        }
        nodes(&value.children, output);
    }
    fn nodes(children: &[Node], output: &mut Vec<String>) {
        for child in children {
            match child {
                Node::Text(text) => output.push(text.value()),
                Node::Element(child) => element(child, output),
                Node::If {
                    then_nodes,
                    else_nodes,
                    ..
                } => {
                    nodes(then_nodes, output);
                    nodes(else_nodes, output);
                }
                Node::For { children, .. } => nodes(children, output),
                Node::Match { arms, .. } => {
                    for arm in arms {
                        nodes(&arm.children, output);
                    }
                }
                Node::Expression(_) => {}
            }
        }
    }
    let mut output = Vec::new();
    element(view.root(), &mut output);
    output
}
