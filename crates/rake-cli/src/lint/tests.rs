//! Covers: syntax rules, spans, comment directives, level overrides, and JSON snapshots.
//! Real: syn, shared view parser, filesystem discovery, serde JSON.
//! Doubles: source strings and temporary projects; no subprocesses or Cargo.
//! Requires: no display, network, or GPU.

use super::*;
use catalogue::{BUTTON, IMAGE, INDEX, REASON, SUPPRESSION, SYNTAX};

fn check(markup: &str) -> Vec<Diagnostic> {
    let text = format!("fn main() {{ rake::view! {{ {markup} }}; }}");

    check_source(Path::new("src/main.rs"), &text, &Levels::new())
}

fn ids(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.rule_id).collect()
}

#[test]
fn button_label_follows_macro_auto_label_contract() {
    for markup in [
        "<button>{icon}</button>",
        "<button><svg path=\"search.svg\"/></button>",
        "<button>{format!(\"Count {count}\")}</button>",
        "<button>Save {count}</button>",
        "<button>if ready { Save } else { Wait }</button>",
        "<button/>",
        "<button>\" \"</button>",
    ] {
        assert_eq!(ids(&check(markup)), [BUTTON], "{markup}");
    }

    for markup in [
        "<button>Save</button>",
        "<button>\"Save\" \"now\"</button>",
        "<button aria-label=\"Search\">{icon}</button>",
        "<button aria-label={label}>{icon}</button>",
        "<Button icon={icon}/>",
        "<ui::Button/>",
    ] {
        assert!(check(markup).is_empty(), "{markup}");
    }
}

#[test]
fn image_alt_accepts_empty_dynamic_and_component_lookalikes() {
    assert_eq!(ids(&check("<img src=\"a\"/>")), [IMAGE]);
    assert_eq!(ids(&check("<img src=\"a\" aria-label=\"A\"/>")), [IMAGE]);

    for markup in [
        "<img src=\"a\" alt=\"\"/>",
        "<img src=\"a\" alt=\"A\"/>",
        "<img src=\"a\" alt={description}/>",
        "<Image src=\"a\"/>",
        "<svg path=\"a\"/>",
    ] {
        assert!(check(markup).is_empty(), "{markup}");
    }
}

#[test]
fn loop_rule_tracks_enumerate_bindings_and_derived_ids() {
    for key in [
        "index",
        "(\"row\", index)",
        "(index as u64)",
        "index + 1",
        "index.to_string()",
        "format!(\"row-{index}\")",
        "format!(\"row-{}\", index)",
        "format!(\"{index:03}\")",
    ] {
        let markup = format!(
            "<div>for (index, row) in rows.iter().enumerate().rev() {{ <div id={{{key}}}/> }}</div>"
        );

        assert_eq!(ids(&check(&markup)), [INDEX], "{markup}");
    }
}

#[test]
fn data_keys_named_i_and_shadowed_indices_do_not_fire() {
    for markup in [
        "<div>for i in &rows { <div id={i.id}/> }</div>",
        "<div>for (index, i) in rows.iter().enumerate() { <div id={i.id}/> }</div>",
        "<div>for (i, row) in rows.iter().enumerate() { <div id={row.i}/> }</div>",
        r#"<div>for (i, row) in rows.iter().enumerate() { <div id={format!("{i}", i = row.id)}/> }</div>"#,
        "<div>for (i, row) in rows.iter().enumerate() { <div id={rows[i].id}/> }</div>",
        "<div>for (i, row) in rows.iter().enumerate() { <div id={format!(\"{{i}}\")}/> }</div>",
        "<div>for (i, row) in rows.iter().enumerate() { for i in row.keys { <div id={i}/> } }</div>",
        "<div>for (i, row) in rows.iter().enumerate() { match row { Some(i) => { <div id={i}/> }, _ => {} } }</div>",
        "<div>for (i, row) in rows.iter().enumerate() { if let Some(i) = row.key { <div id={i}/> } }</div>",
        "<div>for (i, row) in rows.iter().enumerate().map(transform) { <div id={i}/> }</div>",
    ] {
        assert!(check(markup).is_empty(), "{markup}");
    }
}

#[test]
fn loop_rules_visit_branches_and_nested_loops() {
    let markup = "<div>for (i, row) in rows.iter().enumerate() { if row.ready { <div id={i}/> } else { match row.kind { _ => { <div id={i}/> } } } for (j, child) in row.children.enumerate() { <div id={(i,j)}/> } }</div>";

    assert_eq!(ids(&check(markup)), [INDEX, INDEX, INDEX]);
}

#[test]
fn component_slots_and_nested_view_macros_are_checked_once() {
    assert_eq!(
        ids(&check(
            "<Card header={<button>{icon}</button>}>{rake::view! { <img src=\"a\"/> }}</Card>"
        )),
        [BUTTON, IMAGE]
    );
    let source = "fn main() { wrap!(rake::view! { <button/> }); }";

    assert_eq!(
        ids(&check_source(Path::new("main.rs"), source, &Levels::new())),
        [BUTTON]
    );
}

#[test]
fn spans_point_to_original_tags_and_key_expressions() {
    let source = "fn main() {\n    rake::view! {\n        <div>\n            <button>{icon}</button>\n            <img src=\"a\"/>\n            for (i, row) in rows.enumerate() { <div id={(\"row\", i)}/> }\n        </div>\n    };\n}";
    let diagnostics = check_source(Path::new("main.rs"), source, &Levels::new());
    let positions: Vec<_> = diagnostics
        .iter()
        .map(|d| {
            (
                d.span.start.line,
                d.span.start.column,
                d.span.end.line,
                d.span.end.column,
            )
        })
        .collect();

    assert_eq!(positions, [(4, 14, 4, 20), (5, 14, 5, 17), (6, 57, 6, 67)]);
}

#[test]
fn unicode_positions_count_characters() {
    let source = "fn main() { let _ = \"é🙂\"; rake::view! { <button/> }; }";
    let diagnostics = check_source(Path::new("main.rs"), source, &Levels::new());
    let column = source.chars().position(|c| c == '<').unwrap() + 2;

    assert_eq!(diagnostics[0].span.start.column, column);
    assert_eq!(diagnostics[0].span.end.column, column + 6);
}

#[test]
fn json_report_snapshot() {
    let diagnostics = check_source(
        Path::new("src/main.rs"),
        "fn main() { rake::view! { <img src=\"a\"/> }; }",
        &Levels::new(),
    );
    let report = Report {
        schema_version: 1,
        diagnostics,
        files_checked: 1,
        denied: true,
    };
    let json = serde_json::to_string_pretty(&report).unwrap();

    assert_eq!(json, include_str!("snapshots/report.json").trim_end());
}

#[test]
fn syntax_errors_are_deny_and_not_silently_skipped() {
    for source in ["fn {", "fn main() { rake::view! { <div></span> }; }"] {
        let diagnostics = check_source(Path::new("main.rs"), source, &Levels::new());

        assert_eq!(ids(&diagnostics), [SYNTAX], "{source}");
        assert_eq!(diagnostics[0].level, Level::Deny);
    }
}

#[test]
fn suppression_with_reason_targets_same_or_next_code_line() {
    for source in [
        "fn main() { rake::view! { <button/> }; } // rake-lint: allow(view-button-label) Named by widget.",
        "// rake-lint: allow(view-button-label) Named by widget.\n\n// More context\nfn main() { rake::view! { <button/> }; }",
        "fn main() { rake::view! { <div>\n// rake-lint: allow(view-button-label) Named by widget.\n<button/>\n</div> }; }",
    ] {
        assert!(
            check_source(Path::new("main.rs"), source, &Levels::new()).is_empty(),
            "{source}"
        );
    }
}

#[test]
fn missing_reason_warns_and_does_not_suppress() {
    let source =
        "// rake-lint: allow(view-button-label)  \nfn main() { rake::view! { <button/> }; }";
    let diagnostics = check_source(Path::new("main.rs"), source, &Levels::new());

    assert_eq!(ids(&diagnostics), [BUTTON, REASON]);
    assert_eq!(diagnostics[1].span.start.line, 1);
    assert_eq!(diagnostics[1].span.start.column, 1);
}

#[test]
fn invalid_suppressions_warn_and_never_hide_other_rules() {
    for directive in [
        "allow(view-buton-label) Reason.",
        "allow(syntax-error) Reason.",
        "allow(view-button-label,view-image-alt) Reason.",
        "deny(view-button-label) Reason.",
    ] {
        let source =
            format!("// rake-lint: {directive}\nfn main() {{ rake::view! {{ <button/> }}; }}");

        assert_eq!(
            ids(&check_source(Path::new("main.rs"), &source, &Levels::new())),
            [BUTTON, SUPPRESSION]
        );
    }
}

#[test]
fn string_literals_and_nested_block_comments_are_not_directives() {
    for prefix in [
        "let _ = \"// rake-lint: allow(view-button-label) fake\";",
        "let _ = r###\"\n// rake-lint: allow(view-button-label) fake\n\"###;",
        "/* // rake-lint: allow(view-button-label) fake */",
        "/* outer /* inner */ // rake-lint: allow(view-button-label) fake\n */",
        "let _ = r\"/*\"; /* // rake-lint: allow(view-button-label) fake */",
    ] {
        let source = format!("fn main() {{ {prefix}\nrake::view! {{ <button/> }}; }}");

        assert_eq!(
            ids(&check_source(Path::new("main.rs"), &source, &Levels::new())),
            [BUTTON],
            "{source}"
        );
    }
}

#[test]
fn suppression_is_local_and_rule_specific() {
    let source = "// rake-lint: allow(view-button-label) Named by widget.\nfn first() { rake::view! { <button><img src=\"a\"/></button> }; }\nfn second() { rake::view! { <button/> }; }";

    assert_eq!(
        ids(&check_source(Path::new("main.rs"), source, &Levels::new())),
        [IMAGE, BUTTON]
    );
}

#[test]
fn levels_and_discovery_use_nearest_package_config() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();

    fs::write(root.join("Cargo.toml"), "[package]\nname='fixture'\n[package.metadata.rake.lint.levels]\nview-button-label='deny'\nview-image-alt='allow'\n").unwrap();
    fs::write(
        root.join("main.rs"),
        "fn main() { rake::view! { <button><img/></button> }; }",
    )
    .unwrap();
    fs::create_dir(root.join("target")).unwrap();
    fs::write(root.join("target/bad.rs"), "not Rust").unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/Cargo.toml"), "[package]\nname='nested'\n").unwrap();
    fs::write(
        root.join("nested/lib.rs"),
        "fn main() { rake::view! { <button/> }; }",
    )
    .unwrap();

    let report = check_project(root).unwrap();

    assert_eq!(report.files_checked, 2);
    assert!(report.denied);
    assert_eq!(ids(&report.diagnostics), [BUTTON, BUTTON]);
    assert_eq!(report.diagnostics[0].level, Level::Deny);
    assert_eq!(report.diagnostics[1].level, Level::Warn);
}

#[test]
fn config_rejects_typo_levels_rules_and_syntax_override() {
    for config in [
        "view-button-label='oops'",
        "view-typo='warn'",
        "syntax-error='allow'",
    ] {
        let project = tempfile::tempdir().unwrap();

        fs::write(
            project.path().join("Cargo.toml"),
            format!("[package.metadata.rake.lint.levels]\n{config}\n"),
        )
        .unwrap();

        assert!(check_project(project.path()).is_err(), "{config}");
    }
}

#[test]
fn trybuild_stderr_siblings_skip_only_the_matching_source_file() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();

    fs::write(root.join("Cargo.toml"), "[package]\nname='fixture'\n").unwrap();
    fs::create_dir_all(root.join("tests/ui")).unwrap();
    fs::write(root.join("tests/ui/intentional.rs"), "fn {").unwrap();
    fs::write(
        root.join("tests/ui/intentional.stderr"),
        "expected compiler error",
    )
    .unwrap();
    fs::write(root.join("tests/ui/real_error.rs"), "fn {").unwrap();
    fs::write(
        root.join("tests/ui/real_error.rs.stderr"),
        "not the sibling convention",
    )
    .unwrap();
    fs::write(root.join("main.rs"), "fn main() {}").unwrap();

    let report = check_project(root).unwrap();

    assert_eq!(report.files_checked, 2);
    assert!(report.denied);
    assert_eq!(ids(&report.diagnostics), [SYNTAX]);
    assert_eq!(report.diagnostics[0].file, "tests/ui/real_error.rs");
}

#[test]
fn project_excludes_match_files_and_directory_components() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();

    fs::write(root.join("Cargo.toml"), "[package]\nname='fixture'\n[package.metadata.rake.lint]\nexclude=['tests/ui', 'generated.rs']\n").unwrap();
    fs::create_dir_all(root.join("tests/ui/nested")).unwrap();
    fs::create_dir_all(root.join("tests/ui-other")).unwrap();
    fs::write(root.join("tests/ui/nested/fail.rs"), "fn {").unwrap();
    fs::write(root.join("tests/ui-other/error.rs"), "fn {").unwrap();
    fs::write(root.join("generated.rs"), "fn {").unwrap();
    fs::write(root.join("main.rs"), "fn main() {}").unwrap();

    let report = check_project(root).unwrap();

    assert_eq!(report.files_checked, 2);
    assert_eq!(ids(&report.diagnostics), [SYNTAX]);
    assert_eq!(report.diagnostics[0].file, "tests/ui-other/error.rs");
}

#[test]
fn exclusions_remain_relative_to_the_package_that_declares_them() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();

    fs::write(root.join("Cargo.toml"), "[package]\nname='fixture'\n[package.metadata.rake.lint]\nexclude=['ignored', 'nested/shared-ignore']\n").unwrap();
    fs::create_dir_all(root.join("ignored")).unwrap();
    fs::write(root.join("ignored/fail.rs"), "fn {").unwrap();
    fs::create_dir_all(root.join("nested/src/generated")).unwrap();
    fs::create_dir_all(root.join("nested/shared-ignore")).unwrap();
    fs::create_dir_all(root.join("nested/ignored")).unwrap();
    fs::write(
        root.join("nested/Cargo.toml"),
        "[package]\nname='nested'\n[package.metadata.rake.lint]\nexclude=['src/generated']\n",
    )
    .unwrap();
    fs::write(root.join("nested/src/generated/fail.rs"), "fn {").unwrap();
    fs::write(root.join("nested/shared-ignore/fail.rs"), "fn {").unwrap();
    fs::write(root.join("nested/ignored/keep.rs"), "fn main() {}").unwrap();
    fs::write(root.join("nested/src/lib.rs"), "fn main() {}").unwrap();

    let report = check_project(root).unwrap();

    assert_eq!(report.files_checked, 2);
    assert!(report.diagnostics.is_empty());
}

#[test]
fn exclude_config_rejects_unknown_keys_and_non_relative_paths() {
    for config in [
        "excludes=['tests/ui']",
        "exclude=['../outside']",
        "exclude=['/outside']",
        "exclude=['']",
    ] {
        let project = tempfile::tempdir().unwrap();

        fs::write(
            project.path().join("Cargo.toml"),
            format!("[package]\nname='fixture'\n[package.metadata.rake.lint]\n{config}\n"),
        )
        .unwrap();

        assert!(check_project(project.path()).is_err(), "{config}");
    }
}

#[test]
fn terminal_gutter_aligns_single_and_multiple_digit_lines() {
    for (number, arrow, empty, row, underline, note) in [
        (
            3,
            " --> main.rs:3:2",
            "  |",
            "3 | <button/>",
            "  |  ^^^^^^",
            "  = note: ",
        ),
        (
            123,
            "   --> main.rs:123:2",
            "    |",
            "123 | <button/>",
            "    |  ^^^^^^",
            "    = note: ",
        ),
    ] {
        let mut diagnostic =
            Diagnostic::new(BUTTON, Path::new("main.rs"), proc_macro2::Span::call_site());

        diagnostic.span.start = Position {
            line: number,
            column: 2,
        };
        diagnostic.span.end = Position {
            line: number,
            column: 8,
        };
        let source = format!("{}<button/>", "\n".repeat(number - 1));

        let rendered = render(&diagnostic, &source);
        let lines: Vec<_> = rendered.lines().collect();

        assert_eq!(lines[1], arrow);
        assert_eq!(lines[2], empty);
        assert_eq!(lines[3], row);
        assert_eq!(lines[4], underline);
        assert!(lines[5].starts_with(note), "{rendered}");
        assert_eq!(lines[5].find('='), lines[2].find('|'));
        assert_eq!(lines[6].find('='), lines[2].find('|'));
    }
}

#[test]
fn json_paths_use_forward_slashes_for_native_path_components() {
    let path = Path::new("src").join("nested folder").join("日本語.rs");
    let diagnostic = Diagnostic::new(IMAGE, &path, proc_macro2::Span::call_site());
    let json = serde_json::to_value(diagnostic).unwrap();

    assert_eq!(json["file"], "src/nested folder/日本語.rs");
}
