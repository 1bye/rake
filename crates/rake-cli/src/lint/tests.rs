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
        original_sources: BTreeMap::new(),
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

// Syntax-tier additions keep semantic uncertainty diagnostic-only.
#[test]
fn style_rule_accepts_only_exact_pure_utilities() {
    for closure in [
        "|s| s.flex().gap_4().items_center()",
        "|s| { s.p_4().w_full() }",
        "|s| (s.rounded_md())",
    ] {
        let diagnostics = check(&format!(
            "<div id=\"a\" style={{{closure}}} aria-label=\"b\"/>"
        ));
        assert_eq!(ids(&diagnostics), [catalogue::STYLE], "{closure}");
        let edit = diagnostics[0].edit.as_ref().unwrap();
        assert!(edit.replacement.starts_with("class=\""));
        assert!(edit.original.starts_with("style="));
        assert!(edit.original.ends_with('}'));
    }
    for markup in [
        "<div style={|s| s.w(gpui::px(12.))}/>",
        "<div style={|s| s.bg(color)}/>",
        "<div style={|s| s.hover(|s| s.flex())}/>",
        "<div style={|s| { side_effect(); s.flex() }}/>",
        "<div style={move |s| s.flex()}/>",
        "<div style={|mut s| s.flex()}/>",
        "<div style={|s: Div| s.flex()}/>",
        "<div style={|s| s.unknown()}/>",
        "<div style={|s| other.flex()}/>",
        "<div style={|s| s.flex::<T>()}/>",
        "<div class=\"p-2\" style={|s| s.flex()}/>",
        "<Widget style={|s| s.flex()}/>",
    ] {
        assert!(check(markup).is_empty(), "{markup}");
    }
}

#[test]
fn style_comments_are_kept_by_withholding_the_edit_and_suppression_works() {
    let invalid = check("<div style={|s| s.flex()} style={|s| s.p_2()}/>");
    assert_eq!(ids(&invalid), [SYNTAX]);
    assert!(invalid.iter().all(|d| d.edit.is_none()));
    let diagnostics = check("<div style={|s| s /* why flex */ .flex()}/>");
    assert_eq!(ids(&diagnostics), [catalogue::STYLE]);
    assert!(diagnostics[0].edit.is_none());
    assert!(check("\n// rake-lint: allow(view-style-class) Demonstrates the builder API.\n<div style={|s| s.flex()}/>").is_empty());
}

#[test]
fn theme_rule_is_package_evidence_scoped_and_checks_only_design_values() {
    let prefix = "fn install(cx: &mut App) { rake::theme::install(cx, palette); }\n";
    for markup in [
        "<div class=\"bg-[#123456]\"/>",
        "<div class=\"hover:text-[#112233] rounded-[4px]\"/>",
        "<p class={if ready { \"text-[13px]\" } else { \"text-body\" }}>Hello</p>",
        "<div class:bg-[#112233]={active}/>",
        "<div style={|s| s.bg(gpui::rgb(0x112233))}/>",
        "<div style={|s| s.text_size(px(13.)).rounded(rems(0.5))}/>",
    ] {
        let text = format!("{prefix}fn main() {{ rake::view! {{ {markup} }}; }}");
        let diagnostics = check_source(Path::new("main.rs"), &text, &Levels::new());
        assert!(!diagnostics.is_empty(), "{markup}");
        assert!(
            diagnostics
                .iter()
                .all(|d| d.rule_id == catalogue::THEME && d.edit.is_none())
        );
        assert!(check(markup).is_empty(), "no theme: {markup}");
    }
    for markup in [
        "<div class=\"bg-surface text-body rounded-control p-4 w-[100px]\"/>",
        "<div style={|s| s.bg(colors.surface).text_size(theme.body)}/>",
        "<div style={|s| s.w(px(100.))}/>",
        "<div style={|s| other.text_size(px(13.))}/>",
        "<Widget class=\"bg-[#123456]\"/>",
        "<div class={if condition(\"bg-[#123456]\") { \"bg-surface\" } else { \"bg-ink\" }}/>",
        "<div class:rounded-control={active}/>",
    ] {
        let text = format!("{prefix}fn main() {{ rake::view! {{ {markup} }}; }}");
        assert!(
            check_source(Path::new("main.rs"), &text, &Levels::new()).is_empty(),
            "{markup}"
        );
    }
    let text = format!(
        "{prefix}fn main() {{ rake::view! {{\n// rake-lint: allow(view-theme-literal) Intentional palette swatch.\n<div class=\"bg-[#112233]\"/> }}; }}"
    );
    assert!(check_source(Path::new("main.rs"), &text, &Levels::new()).is_empty());
}

#[test]
fn theme_evidence_ignores_lookalikes_strings_and_aliases() {
    for text in [
        "fn f() { rake::theme::install(cx, colors); }",
        "fn f() { ::rake::theme::install(cx, colors); }",
        "fn f() { rake::App::new(\"x\").routes(routes).theme(colors); }",
    ] {
        assert!(source::installs_theme(text), "{text}");
    }
    for text in [
        "fn f() { other::theme::install(cx, colors); }",
        "fn f() { other::App::new(\"x\").theme(colors); }",
        "fn f() { app.theme(colors); }",
        "const S: &str = \"rake::theme::install(cx, colors)\";",
        "fn f() { theme::install(cx, colors); }",
    ] {
        assert!(!source::installs_theme(text), "{text}");
    }
}

#[test]
fn theme_evidence_crosses_source_files_but_not_package_boundaries() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='app'\nversion='0.1.0'",
    )
    .unwrap();
    fs::write(
        root.join("install.rs"),
        "fn install() { rake::theme::install(cx, colors); }",
    )
    .unwrap();
    fs::write(
        root.join("view.rs"),
        "fn view() { rake::view! { <div class=\"bg-[#123456]\"/> }; }",
    )
    .unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(
        root.join("nested/Cargo.toml"),
        "[package]\nname='nested'\nversion='0.1.0'",
    )
    .unwrap();
    fs::write(
        root.join("nested/view.rs"),
        "fn view() { rake::view! { <div class=\"bg-[#123456]\"/> }; }",
    )
    .unwrap();

    let report = check_project(root).unwrap();

    assert_eq!(ids(&report.diagnostics), [catalogue::THEME]);
    assert_eq!(report.diagnostics[0].file, "view.rs");
}

#[test]
fn manifest_rule_handles_release_inheritance_and_explicit_overrides() {
    let text = "[profile.dev]\ndebug-assertions = true\n[profile.release]\ndebug-assertions = true # retain this comment\n[profile.ship]\ninherits = 'release'\ndebug-assertions = true\n[profile.ship.package.foo]\ndebug-assertions = true\n[profile.ship.build-override]\ndebug-assertions = true\n[profile.other]\ninherits='dev'\ndebug-assertions=true\n";
    let diagnostics = manifest::check(Path::new("Cargo.toml"), text).unwrap();
    assert_eq!(ids(&diagnostics), [catalogue::RELEASE; 4]);
    let mut fixed = text.to_owned();
    let mut edits: Vec<_> = diagnostics
        .iter()
        .map(|d| d.edit.as_ref().unwrap())
        .collect();
    edits.sort_by_key(|e| std::cmp::Reverse(e.range.start));
    for edit in edits {
        assert_eq!(edit.original, "true");
        fixed.replace_range(edit.range.clone(), &edit.replacement);
    }
    let before: toml::Value = toml::from_str(text).unwrap();
    let after: toml::Value = toml::from_str(&fixed).unwrap();
    assert_eq!(before["profile"]["dev"], after["profile"]["dev"]);
    assert_eq!(before["profile"]["other"], after["profile"]["other"]);
    assert!(fixed.contains("false # retain this comment"));
    assert!(
        manifest::check(Path::new("Cargo.toml"), &fixed)
            .unwrap()
            .is_empty()
    );
    for text in [
        "[features]\ndevtools=['rake/devtools']\n[profile.release]\ndebug-assertions=false",
        "[dependencies]\nrake={version='1', features=['devtools']}\n",
        "[profile.dev]\ndebug-assertions=true",
        "[profile.a]\ninherits='b'\ndebug-assertions=true\n[profile.b]\ninherits='a'",
        "[profile.release]\nopt-level=3",
    ] {
        assert!(
            manifest::check(Path::new("Cargo.toml"), text)
                .unwrap()
                .is_empty(),
            "{text}"
        );
    }
}

#[test]
fn manifest_suppressions_require_reasons_and_ignore_all_toml_string_forms() {
    for directive in [
        "# rake-lint: allow(release-debug-assertions) Dedicated diagnostics artifact.\ndebug-assertions=true",
        "debug-assertions=true # rake-lint: allow(release-debug-assertions) Dedicated diagnostics artifact.",
    ] {
        let text = format!("[profile.release]\n{directive}\n");
        let diagnostics = manifest::check(Path::new("Cargo.toml"), &text).unwrap();
        assert!(
            filter_diagnostics(
                Path::new("Cargo.toml"),
                &text,
                &Levels::new(),
                diagnostics,
                true
            )
            .is_empty()
        );
    }
    let text =
        "[profile.release]\n# rake-lint: allow(release-debug-assertions)\ndebug-assertions=true\n";
    let diagnostics = manifest::check(Path::new("Cargo.toml"), text).unwrap();
    assert_eq!(
        ids(&filter_diagnostics(
            Path::new("Cargo.toml"),
            text,
            &Levels::new(),
            diagnostics,
            true
        )),
        [catalogue::RELEASE, REASON]
    );
    for value in [
        "'# rake-lint: allow(release-debug-assertions) no'",
        "\"# rake-lint: allow(release-debug-assertions) no\"",
        "'''\n# rake-lint: allow(release-debug-assertions) no\n'''",
        "\"\"\"\n# rake-lint: allow(release-debug-assertions) no\n\"\"\"",
    ] {
        let text = format!("[package]\nname={value}\n[profile.release]\ndebug-assertions=true");
        let diagnostics = manifest::check(Path::new("Cargo.toml"), &text).unwrap();
        assert_eq!(
            ids(&filter_diagnostics(
                Path::new("Cargo.toml"),
                &text,
                &Levels::new(),
                diagnostics,
                true
            )),
            [catalogue::RELEASE]
        );
    }
}

#[test]
fn manifest_fixes_obey_levels_even_without_rust_files() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let text =
        "[package]\nname='app'\nversion='0.1.0'\n[profile.release]\ndebug-assertions=true # why\n";
    fs::write(root.join("Cargo.toml"), text).unwrap();
    let report = check_project(root).unwrap();
    assert_eq!(report.files_checked, 0);
    assert_eq!(ids(&report.diagnostics), [catalogue::RELEASE]);

    fix::apply(root, &report).unwrap();
    let fixed = fs::read_to_string(root.join("Cargo.toml")).unwrap();
    assert!(fixed.contains("debug-assertions=false # why"));
    let report = check_project(root).unwrap();
    assert!(report.diagnostics.is_empty());
    fix::apply(root, &report).unwrap();
    assert_eq!(fs::read_to_string(root.join("Cargo.toml")).unwrap(), fixed);

    fs::write(
        root.join("Cargo.toml"),
        format!("{text}\n[package.metadata.rake.lint.levels]\nrelease-debug-assertions='allow'"),
    )
    .unwrap();
    assert!(check_project(root).unwrap().diagnostics.is_empty());
}

#[test]
fn stale_fixes_do_not_overwrite_changed_sources() {
    let project = tempfile::tempdir().unwrap();
    fs::write(
        project.path().join("Cargo.toml"),
        "[profile.release]\ndebug-assertions=true",
    )
    .unwrap();
    let report = check_project(project.path()).unwrap();
    let changed = "[profile.release]\ndebug-assertions=false # changed concurrently";
    fs::write(project.path().join("Cargo.toml"), changed).unwrap();

    assert!(fix::apply(project.path(), &report).is_err());
    assert_eq!(
        fs::read_to_string(project.path().join("Cargo.toml")).unwrap(),
        changed
    );
}

#[test]
fn catalogue_artifacts_are_current() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert_eq!(
        fs::read_to_string(root.join(".agents/skills/rake-gpui/SKILL.md")).unwrap(),
        guide::generate(),
        "regenerate with cargo run -p rake-app-cli -- lint --guide > .agents/skills/rake-gpui/SKILL.md"
    );
    let agents = fs::read_to_string(root.join("AGENTS.md")).unwrap();
    assert_eq!(
        guide::refresh_agents(&agents).unwrap(),
        agents,
        "refresh root AGENTS.md with --guide=agents --update"
    );
    assert_eq!(
        fs::read_to_string(root.join("crates/rake-cli/templates/default/AGENTS.md")).unwrap(),
        guide::agents()
    );
    assert_eq!(
        fs::read_to_string(root.join("crates/rake-cli/templates/default/CLAUDE.md")).unwrap(),
        "@AGENTS.md\n"
    );
    let docs = fs::read_to_string(root.join("docs/lint.md")).unwrap();
    let generated = docs
        .split("<!-- lint-rules:begin -->\n")
        .nth(1)
        .unwrap()
        .split("<!-- lint-rules:end -->")
        .next()
        .unwrap();
    assert_eq!(
        generated,
        guide::rules(),
        "update the generated rule table in docs/lint.md from guide::rules()"
    );
}

#[test]
fn syntax_errors_prevent_edits_in_the_same_file() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='app'\nversion='0.1.0'",
    )
    .unwrap();
    let source =
        "fn f() { rake::view! { <div style={|s| s.flex()}/> }; rake::view! { <div id=123/> }; }";
    fs::write(root.join("main.rs"), source).unwrap();

    let report = check_project(root).unwrap();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.rule_id == catalogue::STYLE && d.edit.is_some())
    );
    assert!(report.diagnostics.iter().any(|d| d.rule_id == SYNTAX));
    fix::apply(root, &report).unwrap();
    assert_eq!(fs::read_to_string(root.join("main.rs")).unwrap(), source);
}

#[test]
fn every_style_fix_lowers_to_the_identical_ordered_method_chain() {
    let source = "fn f() { rake::view! { <div aria-label=\"before\" style={|s| s.p_4().px_2().p_0().flex()} id=\"after\"/> }; }";
    let diagnostics = check_source(Path::new("main.rs"), source, &Levels::new());
    let edit = diagnostics[0].edit.as_ref().unwrap();
    let mut fixed = source.to_owned();
    fixed.replace_range(edit.range.clone(), &edit.replacement);
    assert!(fixed.contains("aria-label=\"before\" class=\"p-4 px-2 p-0 flex\" id=\"after\""));

    // Lower both views using the actual macro compiler. The only executable
    // difference is identity closure application versus its verbatim method chain.
    let original_view: rake_view_syntax::View =
        syn::parse_str("<div style={|s| s.p_4().px_2().p_0().flex()}/>").unwrap();
    let fixed_view: rake_view_syntax::View =
        syn::parse_str("<div class=\"p-4 px-2 p-0 flex\"/>").unwrap();
    let rake: proc_macro2::TokenStream = "::rake".parse().unwrap();
    let original = original_view.expand_resilient(&rake).unwrap().to_string();
    let fixed = fixed_view.expand_resilient(&rake).unwrap().to_string();
    assert!(original.contains(". map (| s | s . p_4 () . px_2 () . p_0 () . flex ())"));
    assert!(fixed.contains(". p_4 () . px_2 () . p_0 () . flex ()"));
    syn::parse_str::<syn::Expr>(&original).unwrap();
    syn::parse_str::<syn::Expr>(&fixed).unwrap();
}

#[test]
fn context_changes_invalidate_fixes_even_when_the_attribute_is_unchanged() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='app'\nversion='0.1.0'",
    )
    .unwrap();
    let original = "fn f() { rake::view! { <div style={|s| s.flex()}/> }; }";
    fs::write(root.join("main.rs"), original).unwrap();
    let report = check_project(root).unwrap();
    let changed = original.replace("<div", "<Foo");
    fs::write(root.join("main.rs"), &changed).unwrap();

    assert!(fix::apply(root, &report).is_err());
    assert_eq!(fs::read_to_string(root.join("main.rs")).unwrap(), changed);
}

#[test]
fn file_suppressions_cover_matching_findings_without_hiding_other_rules() {
    let source = "//! Header docs\n/* header block */\n// rake-lint: allow-file(view-button-label) Mounted widgets supply action names.\nfn f() { rake::view! { <div><button/><button/><img src=\"x\"/></div> }; }";
    let diagnostics = check_source(Path::new("main.rs"), source, &Levels::new());
    assert_eq!(ids(&diagnostics), [IMAGE]);

    let other_file = check_source(
        Path::new("other.rs"),
        "fn f() { rake::view! { <button/> }; }",
        &Levels::new(),
    );
    assert_eq!(ids(&other_file), [BUTTON]);
}

#[test]
fn file_suppressions_require_reasons_and_valid_header_placement() {
    for directive in [
        "// rake-lint: allow-file(view-button-label)",
        "// rake-lint: allow-file(view-button-label)   ",
    ] {
        let source = format!("{directive}\nfn f() {{ rake::view! {{ <button/> }}; }}");
        assert_eq!(
            ids(&check_source(Path::new("main.rs"), &source, &Levels::new())),
            [BUTTON, REASON]
        );
    }
    for source in [
        "// rake-lint: allow-file(unknown) Reason.\nfn f() {}",
        "// rake-lint: allow-file(syntax-error) Reason.\nfn f() {}",
        "// rake-lint: allow-file(invalid-suppression) Reason.\nfn f() {}",
        "// rake-lint: allow-file(view-button-label Reason.\nfn f() {}",
        "use std::fmt;\n// rake-lint: allow-file(view-button-label) Too late.\nfn f() {}",
        "fn f() {} // rake-lint: allow-file(view-button-label) Trailing.",
        "fn f() {\n// rake-lint: allow-file(view-button-label) Nested.\nrake::view! { <button/> }; }",
    ] {
        let diagnostics = check_source(Path::new("main.rs"), source, &Levels::new());
        assert!(
            diagnostics.iter().any(|d| d.rule_id == SUPPRESSION),
            "{source}"
        );
        if source.contains("<button/>") {
            assert!(diagnostics.iter().any(|d| d.rule_id == BUTTON));
        }
    }
}

#[test]
fn file_suppressions_report_unused_and_duplicate_directives_at_their_comments() {
    let source = "// rake-lint: allow-file(view-button-label) No buttons here.\nfn f() {}";
    let diagnostics = check_source(Path::new("main.rs"), source, &Levels::new());
    assert_eq!(ids(&diagnostics), [SUPPRESSION]);
    assert!(
        diagnostics[0]
            .message
            .contains("unused file suppression for view-button-label")
    );
    assert_eq!(diagnostics[0].span.start.line, 1);
    assert_eq!(diagnostics[0].span.start.column, 1);
    assert_eq!(
        diagnostics[0].span.end.column,
        source.lines().next().unwrap().chars().count() + 1
    );

    let source = "// rake-lint: allow-file(view-button-label) First reason.\n// rake-lint: allow-file(view-button-label) Redundant reason.\nfn f() { rake::view! { <button/> }; }";
    let diagnostics = check_source(Path::new("main.rs"), source, &Levels::new());
    assert_eq!(ids(&diagnostics), [SUPPRESSION]);
    assert!(diagnostics[0].message.contains("unused file suppression"));
    assert_eq!(diagnostics[0].span.start.line, 2);
}

#[test]
fn file_suppression_usage_respects_effective_levels() {
    let source = "// rake-lint: allow-file(view-button-label) Embedded widget.\nfn f() { rake::view! { <button/> }; }";
    let levels = BTreeMap::from([(BUTTON.into(), Level::Allow)]);
    let diagnostics = check_source(Path::new("main.rs"), source, &levels);
    assert_eq!(ids(&diagnostics), [SUPPRESSION]);

    let source = "// rake-lint: allow-file(view-button-label) No buttons.\nfn f() {}";
    for level in [Level::Warn, Level::Deny] {
        let diagnostics = check_source(
            Path::new("main.rs"),
            source,
            &BTreeMap::from([(SUPPRESSION.into(), level)]),
        );
        assert_eq!(diagnostics[0].level, level);
    }
    assert!(
        check_source(
            Path::new("main.rs"),
            source,
            &BTreeMap::from([(SUPPRESSION.into(), Level::Allow)])
        )
        .is_empty()
    );
}

#[test]
fn file_suppressions_ignore_strings_docs_and_nested_block_comments() {
    for prefix in [
        "//! Header docs\nconst S: &str = r#\"// rake-lint: allow-file(view-button-label) Fake.\"#;",
        "const S: &str = \"// rake-lint: allow-file(view-button-label) Fake.\";",
        "const S: &str = r#\"// rake-lint: allow-file(view-button-label) Fake.\"#;",
        "/// rake-lint: allow-file(view-button-label) Fake.",
        "//! rake-lint: allow-file(view-button-label) Fake.",
        "/* outer /* // rake-lint: allow-file(view-button-label) Fake. */ */",
    ] {
        let source = format!("{prefix}\nfn f() {{ rake::view! {{ <button/> }}; }}");
        assert_eq!(
            ids(&check_source(Path::new("main.rs"), &source, &Levels::new())),
            [BUTTON],
            "{prefix}"
        );
    }
}

#[test]
fn file_suppressions_support_toml_headers_with_the_same_usage_checks() {
    let file = Path::new("Cargo.toml");
    let source = "# rake-lint: allow-file(release-debug-assertions) Dedicated diagnostics artifact.\n[profile.release]\ndebug-assertions=true # keep\n";
    let diagnostics = manifest::check(file, source).unwrap();
    assert!(filter_diagnostics(file, source, &Levels::new(), diagnostics, true).is_empty());

    let source = source.replace("=true", "=false");
    let diagnostics = manifest::check(file, &source).unwrap();
    let diagnostics = filter_diagnostics(file, &source, &Levels::new(), diagnostics, true);
    assert_eq!(ids(&diagnostics), [SUPPRESSION]);
    assert!(diagnostics[0].message.contains("unused file suppression"));

    let source = "[profile.release]\n# rake-lint: allow-file(release-debug-assertions) Too late.\ndebug-assertions=true";
    let diagnostics = manifest::check(file, source).unwrap();
    assert_eq!(
        ids(&filter_diagnostics(
            file,
            source,
            &Levels::new(),
            diagnostics,
            true
        )),
        [catalogue::RELEASE, SUPPRESSION]
    );
}

#[test]
fn file_suppression_does_not_leak_to_other_files_in_a_themed_package() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='app'\nversion='0.1.0'",
    )
    .unwrap();
    fs::write(
        root.join("theme.rs"),
        "fn install() { rake::theme::install(cx, colors); }",
    )
    .unwrap();
    let view = "fn view() { rake::view! { <div class=\"text-[10px]\"/> }; }";
    fs::write(
        root.join("allowed.rs"),
        format!(
            "// rake-lint: allow-file(view-theme-literal) Compact inspector typography.\n{view}"
        ),
    )
    .unwrap();
    fs::write(root.join("other.rs"), view).unwrap();

    let report = check_project(root).unwrap();

    assert_eq!(ids(&report.diagnostics), [catalogue::THEME]);
    assert_eq!(report.diagnostics[0].file, "other.rs");
}

#[test]
fn agent_block_refresh_preserves_surroundings_and_refuses_ambiguous_markers() {
    let original = format!(
        "User notes café\r\n{}\r\nOld rules\r\n{}\r\nKeep footer\r\n",
        guide::BEGIN,
        guide::END
    );
    let updated = guide::refresh_agents(&original).unwrap();
    assert!(updated.starts_with("User notes café\r\n"));
    assert!(updated.ends_with("Keep footer\r\n"));
    assert_eq!(guide::refresh_agents(&updated).unwrap(), updated);
    assert!(updated.contains("Never derive loop IDs from positions"));
    assert!(!updated.replace("\r\n", "").contains('\n'));
    for text in [
        "",
        guide::BEGIN,
        &format!("{}\n{}", guide::END, guide::BEGIN),
        &format!("{}\n{}\n{}", guide::BEGIN, guide::BEGIN, guide::END),
        &format!("{}\n{}\n{}", guide::BEGIN, guide::END, guide::END),
        &format!("inline {}\n{}", guide::BEGIN, guide::END),
        &format!("{}\n{} suffix", guide::BEGIN, guide::END),
    ] {
        assert!(guide::refresh_agents(text).is_err(), "{text}");
    }
}

#[test]
fn agent_summaries_are_compact_imperatives_for_actionable_rules() {
    let block = guide::agents();
    assert!(block.lines().count() <= 16);
    assert_eq!(block.matches(guide::BEGIN).count(), 1);
    assert_eq!(block.matches(guide::END).count(), 1);
    for entry in catalogue::RULES {
        if let Some(summary) = entry.agent_summary {
            assert!(!summary.contains('\n'));
            assert!(summary.starts_with("Never ") || summary.starts_with("Always "));
            assert_eq!(block.matches(summary).count(), 1);
        }
    }
    assert!(catalogue::rule(SYNTAX).unwrap().agent_summary.is_none());
    assert!(
        catalogue::rule(SUPPRESSION)
            .unwrap()
            .agent_summary
            .is_none()
    );
}

#[test]
fn single_file_lint_uses_package_theme_evidence_without_visiting_nested_packages() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(root.join("Cargo.toml"), "[package]\nname='app'\nversion='0.1.0'\n[package.metadata.rake.lint.levels]\nview-theme-literal='deny'").unwrap();
    fs::write(
        root.join("theme.rs"),
        "fn install() { rake::theme::install(cx, colors); }",
    )
    .unwrap();
    fs::write(
        root.join("view.rs"),
        "fn f() { rake::view! { <div class=\"bg-[#123456]\"/> }; }",
    )
    .unwrap();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/Cargo.toml"), "malformed nested package").unwrap();
    let (_, report) = check_target(&root.join("view.rs")).unwrap();
    assert_eq!(report.files_checked, 1);
    assert_eq!(ids(&report.diagnostics), [catalogue::THEME]);
    assert!(report.denied);
    assert_eq!(report.diagnostics[0].file, "view.rs");
}

#[test]
fn single_file_lint_respects_exclusions_and_compile_fail_fixtures() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(root.join("Cargo.toml"), "[package]\nname='app'\nversion='0.1.0'\n[package.metadata.rake.lint]\nexclude=['generated.rs']").unwrap();
    for name in ["generated.rs", "fixture.rs"] {
        fs::write(root.join(name), "fn {").unwrap();
    }
    fs::write(root.join("fixture.stderr"), "intentional failure").unwrap();
    for name in ["generated.rs", "fixture.rs"] {
        let (_, report) = check_target(&root.join(name)).unwrap();
        assert_eq!(report.files_checked, 0);
        assert!(report.diagnostics.is_empty());
    }
}
