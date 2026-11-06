//! Rustfmt process integration and project configuration.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use syn::spanned::Spanned;

use super::Result;
use super::source::{atoms, offset, project, span_range};

pub(super) struct Rustfmt {
    pub width: usize,
    pub tab_spaces: usize,
    pub hard_tabs: bool,
    pub edition: String,
    config: Option<PathBuf>,
    cache: HashMap<(String, usize, bool), String>,
}

impl Rustfmt {
    pub fn load(directory: &Path) -> Result<Self> {
        let config = directory.ancestors().find_map(|directory| {
            ["rustfmt.toml", ".rustfmt.toml"]
                .into_iter()
                .map(|name| directory.join(name))
                .find(|path| path.is_file())
        });
        let value: toml::Value = if let Some(path) = &config {
            toml::from_str(&std::fs::read_to_string(path)?)?
        } else {
            toml::Value::Table(Default::default())
        };
        let number = |name, default| {
            value
                .get(name)
                .and_then(toml::Value::as_integer)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(default)
        };
        let width = number("max_width", 100);
        let tab_spaces = number("tab_spaces", 4);
        if width == 0 || tab_spaces == 0 {
            return Err("rustfmt max_width and tab_spaces must be positive".into());
        }
        let mut edition = value
            .get("edition")
            .and_then(toml::Value::as_str)
            .map(str::to_owned);
        if edition.is_none() {
            for ancestor in directory.ancestors() {
                let manifest = ancestor.join("Cargo.toml");
                if !manifest.is_file() {
                    continue;
                }
                let manifest: toml::Value = toml::from_str(&std::fs::read_to_string(manifest)?)?;
                let package = manifest
                    .get("package")
                    .and_then(|package| package.get("edition"))
                    .and_then(toml::Value::as_str);
                let workspace = manifest
                    .get("workspace")
                    .and_then(|workspace| workspace.get("package"))
                    .and_then(|package| package.get("edition"))
                    .and_then(toml::Value::as_str);
                if let Some(value) = package.or(workspace) {
                    edition = Some(value.to_owned());
                    break;
                }
            }
        }
        Ok(Self {
            width,
            tab_spaces,
            hard_tabs: value
                .get("hard_tabs")
                .and_then(toml::Value::as_bool)
                .unwrap_or(false),
            edition: edition.unwrap_or_else(|| "2015".into()),
            config,
            cache: HashMap::new(),
        })
    }

    pub fn indent(&self, columns: usize) -> String {
        if self.hard_tabs {
            format!(
                "{}{}",
                "\t".repeat(columns / self.tab_spaces),
                " ".repeat(columns % self.tab_spaces)
            )
        } else {
            " ".repeat(columns)
        }
    }

    pub fn columns(&self, text: &str) -> usize {
        text.chars().fold(0, |column, character| {
            if character == '\t' {
                column + self.tab_spaces - column % self.tab_spaces
            } else {
                column + 1
            }
        })
    }

    pub fn run(&self, source: &str, width: usize) -> Result<String> {
        let mut command = Command::new("rustfmt");
        command.args(["--emit", "stdout", "--edition", &self.edition]);
        if let Some(path) = &self.config {
            command.arg("--config-path").arg(path);
        }
        command.arg("--config").arg(format!("max_width={width}"));
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut stdin = child.stdin.take().ok_or("rustfmt stdin unavailable")?;
        // Drain pipes while writing large editor buffers, avoiding pipe-capacity deadlocks.
        let input = source.as_bytes().to_owned();
        let writer = std::thread::spawn(move || stdin.write_all(&input));
        let output = child.wait_with_output()?;
        writer
            .join()
            .map_err(|_| "rustfmt input writer panicked")??;
        if !output.status.success() {
            return Err(format!(
                "rustfmt failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
        Ok(super::source::preserve_views(
            source,
            &String::from_utf8(output.stdout)?,
        ))
    }

    pub fn rust(&mut self, source: &str, indent: usize, header: bool) -> Result<String> {
        let key = (source.to_owned(), indent, header);
        if let Some(value) = self.cache.get(&key) {
            return Ok(value.clone());
        }
        let normalized = self.normalize(source);
        let input = normalized.as_str();
        let wrapped = if header {
            if source.trim_end().ends_with("=>") {
                format!("fn __rake_format() {{ match __value {{ {input} {{}} }} }}")
            } else {
                format!("fn __rake_format() {{ {input} {{}} }}")
            }
        } else {
            format!("fn __rake_format() {{ let __value = {input}; }}")
        };
        // The wrapper contributes one indentation level; reserve the actual column
        // budget of the markup location without double-counting that level.
        let width = self
            .width
            .saturating_sub(indent)
            .saturating_add(self.tab_spaces)
            .max(1);
        // Establish standard Rust indentation before applying the narrower markup
        // budget. Rustfmt can leave an expression untouched when it cannot fit.
        let wrapped = if width < self.width {
            self.run(&wrapped, self.width)?
        } else {
            wrapped
        };
        let formatted = self.run(&wrapped, width)?;
        let file = syn::parse_file(&formatted)?;
        let syn::Item::Fn(function) = &file.items[0] else {
            return Err("rustfmt wrapper missing".into());
        };
        let mut candidate = if header {
            let syn::Stmt::Expr(expression, _) = &function.block.stmts[0] else {
                return Err("rustfmt header missing".into());
            };
            let start = offset(&formatted, expression.span().start());
            let end = match expression {
                syn::Expr::If(value) => offset(
                    &formatted,
                    value.then_branch.brace_token.span.open().start(),
                ),
                syn::Expr::ForLoop(value) => {
                    offset(&formatted, value.body.brace_token.span.open().start())
                }
                syn::Expr::Match(value) if source.trim_end().ends_with("=>") => {
                    let arm = &value.arms[0];
                    let start = offset(&formatted, arm.pat.span().start());
                    let end = offset(&formatted, arm.body.span().start());
                    let text = formatted[start..end].trim_end();
                    return Ok(self.finish_rust(source, text, indent));
                }
                syn::Expr::Match(value) => {
                    offset(&formatted, value.brace_token.span.open().start())
                }
                _ => return Ok(source.into()),
            };
            formatted[start..end].trim_end().to_owned()
        } else {
            let syn::Stmt::Local(local) = &function.block.stmts[0] else {
                return Err("rustfmt expression missing".into());
            };
            let expression = &local
                .init
                .as_ref()
                .ok_or("rustfmt initializer missing")?
                .expr;
            formatted[span_range(&formatted, expression.span(), expression.span())].to_owned()
        };
        if !header
            && !candidate.contains('\n')
            && candidate.starts_with("{ ")
            && candidate.ends_with(" }")
        {
            candidate = format!("{{{}}}", &candidate[2..candidate.len() - 2]);
        }
        let value = self.finish_rust(source, &candidate, indent);
        self.cache.insert(key, value.clone());
        Ok(value)
    }

    fn normalize(&self, source: &str) -> String {
        let Some(parts) = atoms(source) else {
            return source.into();
        };
        let mut previous = 0;
        let mut baseline = usize::MAX;
        for part in &parts {
            let gap = &source[previous..part.range.start];
            if let Some((_, tail)) = gap.rsplit_once('\n') {
                baseline = baseline.min(self.columns(tail));
            }
            previous = part.range.end;
        }
        if baseline == usize::MAX {
            return source.into();
        }
        let mut output = String::new();
        previous = 0;
        for part in parts {
            let gap = &source[previous..part.range.start];
            if let Some((before, tail)) = gap.rsplit_once('\n') {
                output.push_str(before);
                output.push('\n');
                output.push_str(
                    &self.indent(self.columns(tail).saturating_sub(baseline) + self.tab_spaces),
                );
            } else {
                output.push_str(gap);
            }
            output.push_str(part.text);
            previous = part.range.end;
        }
        output
    }

    fn finish_rust(&self, source: &str, candidate: &str, indent: usize) -> String {
        // Adjust whitespace gaps only: multiline literals and block comments retain
        // every byte, including indentation inside their contents.
        let Some(parts) = atoms(candidate) else {
            return source.into();
        };
        let mut adjusted = String::new();
        let mut previous = 0;
        for part in parts {
            let gap = &candidate[previous..part.range.start];
            if let Some((before, tail)) = gap.rsplit_once('\n') {
                adjusted.push_str(before);
                adjusted.push('\n');
                let columns = self.columns(tail).saturating_sub(self.tab_spaces) + indent;
                adjusted.push_str(&self.indent(columns));
            } else {
                adjusted.push_str(gap);
            }
            adjusted.push_str(part.text);
            previous = part.range.end;
        }
        super::source::preserve_views(source, &project(source, &adjusted))
    }
}
