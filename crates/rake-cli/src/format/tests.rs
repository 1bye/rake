//! Covers: markup layout, source preservation, and repository-wide fixed points.
//! Real: shared parser, source renderer, installed rustfmt, repository sources.
//! Doubles: none.
//! Requires: pinned rustfmt on PATH; no GPU.

use std::path::Path;

use super::{markup::format_source, rustfmt::Rustfmt, source};

fn formatted(source: &str) -> String {
    let mut rustfmt = Rustfmt::load(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let (output, problems) = format_source(source, &mut rustfmt).unwrap();
    assert!(
        problems.is_empty(),
        "{:?}",
        problems.iter().map(|p| &p.message).collect::<Vec<_>>()
    );
    assert!(source::equivalent(source, &output), "{source}\n{output}");
    let (again, problems) = format_source(&output, &mut rustfmt).unwrap();
    assert!(problems.is_empty());
    assert_eq!(output, again, "not idempotent:\n{source}");
    output
}

#[test]
fn tags_children_and_attribute_order() {
    let output = formatted(
        r#"fn main() { view!{ <div style={|s|s.p_4()} class="px-3   rounded-md" id="root" aria-label="例"><span>"Saved"</span><img src="x" alt="y"/></div> }; }"#,
    );
    assert!(output.contains("<div\n"));
    assert!(output.contains("\n        style={|s| s.p_4()}\n        class=\"px-3   rounded-md\"\n        id=\"root\"\n        aria-label=\"例\"\n    >"), "{output}");
    assert!(output.contains("<span>\"Saved\"</span>"));
    assert!(output.contains("<img src=\"x\" alt=\"y\" />"));
}

#[test]
fn comments_and_multiline_literals_are_indivisible() {
    let source = r####"fn main() { view! {
    // before
    <div /* tag */ id /* name */ = /* value */ "x" style={|s| { /* rust */
        let text = r#"first
     second"#; // trailing rust
        s.child(text)
    }}>
        /* nested /* block */ comment */
        <span>"hi"</span> // trailing child
        // before closing
    </div> // after
}; }"####;
    let output = formatted(source);
    assert!(output.contains("r#\"first\n     second\"#"));
    assert!(output.contains("/* nested /* block */ comment */"));
}

#[test]
fn nested_views_properties_and_control_flow() {
    let output = formatted(
        r#"fn main() { rake::view!{<Panel heading={<h1>"Title"</h1>}><div>if let Some(x)=selected {<span>{x}</span>} else if other {<span/>} else {} match value { Some(x) if x>0=>{<span>{x}</span>}, _=>{} } for (i,row) in rows.iter().enumerate(){<p id={row.id}>{row.name}</p>} {items.map(|item| view!{<span id={item.id}>"Inner"</span>})}</div></Panel>}; }"#,
    );
    assert!(output.contains("if let Some(x) = selected {"), "{output}");
    assert!(output.contains("} else if other {"), "{output}");
    assert!(output.contains("Some(x) if x > 0 => {"), "{output}");
    assert!(
        output.contains("for (i, row) in rows.iter().enumerate() {"),
        "{output}"
    );
    assert!(output.contains("view! {\n"), "{output}");
}

#[test]
fn malformed_invocation_is_untouched_and_siblings_still_format() {
    let source = "fn main() { view! { <div><span></div> }; view!{<div><span/></div>}; }";
    let mut rustfmt = Rustfmt::load(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let (output, problems) = format_source(source, &mut rustfmt).unwrap();
    assert_eq!(problems.len(), 1);
    assert!(problems[0].line == 1 && problems[0].column > 1);
    assert!(output.contains("view! { <div><span></div> }"), "{output}");
    assert!(output.contains("<div>\n        <span/>"));
    assert!(source::equivalent(source, &output));
}

#[test]
fn unquoted_text_keeps_lowered_whitespace() {
    let input = r#"fn main() { view! { <p>Hello <strong>world</strong>! " literal " end</p> }; }"#;
    let output = formatted(input);
    fn texts(source: &str) -> Vec<String> {
        fn collect(nodes: &[rake_view_syntax::Node], output: &mut Vec<String>) {
            for node in nodes {
                match node {
                    rake_view_syntax::Node::Text(text) => output.push(text.value()),
                    rake_view_syntax::Node::Element(element) => collect(&element.children, output),
                    _ => {}
                }
            }
        }
        let invocation = crate::view_source::invocations(source).unwrap().remove(0);
        let view: rake_view_syntax::View = syn::parse2(invocation.tokens).unwrap();
        let mut output = Vec::new();
        collect(&view.root().children, &mut output);
        output
    }
    assert_eq!(texts(input), texts(&output));
}

#[test]
fn project_width_tabs_and_unicode() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("rustfmt.toml"),
        "max_width = 60\ntab_spaces = 2\nhard_tabs = true\nedition = \"2024\"\n",
    )
    .unwrap();
    let mut rustfmt = Rustfmt::load(directory.path()).unwrap();
    let input = "fn main() {\n\tview!{<div id=\"ééééééééééééééééééééééééééééééé\"><span>\"x\"</span></div>};\n}";
    let (output, problems) = format_source(input, &mut rustfmt).unwrap();
    assert!(problems.is_empty());
    assert!(output.contains("\n\t\t<div id="), "{output}");
    assert!(
        !output.contains("<div\n"),
        "width must count characters, not UTF-8 bytes: {output}"
    );
    let (again, _) = format_source(&output, &mut rustfmt).unwrap();
    assert_eq!(output, again);
}

#[test]
fn every_repository_view_is_token_equivalent_and_idempotent() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let tracked = std::process::Command::new("git")
        .args(["ls-files", "-z", "*.rs"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(tracked.status.success());
    let tracked = String::from_utf8(tracked.stdout).unwrap();
    let mut count = 0;
    let mut invalid = 0;
    for relative in tracked.split('\0').filter(|path| !path.is_empty()) {
        let path = root.join(relative);
        let source = std::fs::read_to_string(&path).unwrap();
        let invocations = crate::view_source::invocations(&source).unwrap_or_default();
        count += invocations.len();
        if invocations.is_empty() {
            continue;
        }
        let mut rustfmt = Rustfmt::load(path.parent().unwrap()).unwrap();
        let (once, problems) = format_source(&source, &mut rustfmt).unwrap();
        let expected_errors = expected_errors(&source);
        assert_eq!(
            problems.len(),
            expected_errors,
            "{}: {:?}",
            path.display(),
            problems.iter().map(|p| &p.message).collect::<Vec<_>>()
        );
        invalid += expected_errors;
        assert!(
            source::equivalent(&source, &once),
            "tokens/comments changed: {}",
            path.display()
        );
        let before = crate::view_source::invocations(&source).unwrap();
        let after = crate::view_source::invocations(&once).unwrap();
        assert_eq!(before.len(), after.len(), "{}", path.display());
        for (before, after) in before.into_iter().zip(after) {
            let valid = syn::parse2::<rake_view_syntax::View>(before.tokens.clone())
                .and_then(|view| view.layout().map(|_| ()))
                .is_ok();
            if valid {
                syn::parse2::<rake_view_syntax::View>(after.tokens.clone())
                    .unwrap()
                    .layout()
                    .unwrap();
            } else {
                let before_range = source::span_range(&source, before.open, before.close);
                let after_range = source::span_range(&once, after.open, after.close);
                assert_eq!(
                    source[before_range],
                    once[after_range],
                    "invalid invocation changed: {}",
                    path.display()
                );
            }
            assert_eq!(
                token_atoms(before.tokens),
                token_atoms(after.tokens),
                "macro token stream: {}",
                path.display()
            );
        }
        let (twice, problems) = format_source(&once, &mut rustfmt).unwrap();
        assert_eq!(problems.len(), expected_errors);
        assert_eq!(once, twice, "not idempotent: {}", path.display());
    }
    assert!(
        count > 0 && invalid > 0,
        "repository and compile-fail sources must both be covered"
    );
    eprintln!(
        "Verified {count} repository view! invocations, including {invalid} invalid invocations left unchanged"
    );
}

fn token_atoms(tokens: proc_macro2::TokenStream) -> Vec<String> {
    let mut output = Vec::new();
    for token in tokens {
        if let proc_macro2::TokenTree::Group(group) = token {
            output.push(format!("open:{:?}", group.delimiter()));
            output.extend(token_atoms(group.stream()));
            output.push(format!("close:{:?}", group.delimiter()));
        } else {
            output.push(token.to_string());
        }
    }
    output
}

#[test]
fn deeply_indented_rust_keeps_indentation_when_rustfmt_cannot_fit_a_line() {
    let source = format!(
        "fn main() {{ view! {{ {}<button on:click={{cx.listener(|this, _, _, cx| {{
            if let Some((property, value)) = this.style_text.split_once(':') {{
                this.edit_style(Edit::Set {{ declaration: Declaration {{
                    property: property.trim().into(), value: value.trim().trim_end_matches(';').into(), enabled: true,
                }}}});
            }} else {{ this.error = Some(\"Use property: value\".into()); }}
            cx.notify();
        }})}}>\"Apply\"</button>{} }}; }}",
        "<div>".repeat(9),
        "</div>".repeat(9),
    );
    formatted(&source);
}

#[test]
fn nested_invalid_views_are_reported_and_other_macro_bodies_are_opaque() {
    let source = "fn main() { rsx!{<div  id = \"keep\"/>}; view! { <div>{view! { <span>\n        </div> }}</div> }; }";
    let mut rustfmt = Rustfmt::load(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();
    let (output, problems) = format_source(source, &mut rustfmt).unwrap();
    assert_eq!(problems.len(), 1);
    assert!(
        output.contains("view! { <span>\n        </div> }"),
        "{output}"
    );
    assert!(output.contains("rsx!{<div  id = \"keep\"/>}"), "{output}");
    assert!(source::equivalent(source, &output));
}

fn expected_errors(source: &str) -> usize {
    fn errors(invocation: crate::view_source::Invocation) -> usize {
        let mut count = 0;
        crate::view_source::nested_views(invocation.tokens.clone(), &mut |invocation| {
            count += errors(invocation);
        });
        count
            + usize::from(
                syn::parse2::<rake_view_syntax::View>(invocation.tokens)
                    .and_then(|view| view.layout().map(|_| ()))
                    .is_err(),
            )
    }
    crate::view_source::invocations(source)
        .unwrap()
        .into_iter()
        .map(errors)
        .sum()
}
