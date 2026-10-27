//! Covers: markup grammar, checked attributes/utilities, and loop identity diagnostics.
//! Real: syn parser and GPUI code generator.
//! Doubles: none.
//! Requires: no display or GPU.

use quote::quote;

use super::View;

fn expand(source: &str) -> syn::Result<proc_macro2::TokenStream> {
    syn::parse_str::<View>(source)?.expand(&quote!(::renamed))
}

#[test]
fn accepts_nested_markup_and_rust_control_flow() {
    let expanded = expand(
        r#"
        <section id="root" style={|s| s.p_4()}>
            <h1>"Title"</h1>
            if let Some(value) = selected {
                <p>{value}</p>
            } else if count < 2 {
                "Small"
            } else {
                for (index, row) in rows.iter().enumerate() {
                    <div id={("row", index)}><span>{row.clone()}</span></div>
                }
            }
            <img src={source()} alt="Preview" />
            <button onclick={cx.listener(|this, _, _, cx| this.save(cx))}>"Save"</button>
        </section>
    "#,
    )
    .expect("valid markup");

    syn::parse2::<syn::Expr>(expanded).expect("expands to one Rust expression");
}

#[test]
fn reports_bad_markup_at_compile_time() {
    let cases = [
        ("", "expected `<`"),
        ("<div>", "missing closing </div>"),
        ("<div></span>", "expected closing </div>"),
        ("<div/><p/>", "exactly one root"),
        ("<div>unquoted</div>", "quoted text"),
        ("<input/>", "unsupported view element <input>"),
        ("<div id=\"a\" id=\"b\"/>", "duplicate attribute `id`"),
        ("<div title=\"x\"/>", "unsupported attribute `title`"),
        ("<div tabindex=0/>", "quoted strings or {Rust expressions}"),
        ("<div class={classes}/>", "literal utility string"),
        (
            "<div class=\"bg-red-500\"/>",
            "unsupported utility class `bg-red-500`",
        ),
        ("<img alt=\"x\"/>", "img requires src"),
        ("<img src=\"x\"/>", "img requires alt"),
        (
            "<img src=\"x\" alt=\"x\">\"bad\"</img>",
            "img cannot have children",
        ),
        (
            "<div>for x in xs { <p>{x}</p> }</div>",
            "explicit stable id",
        ),
        (
            "<div>for x in xs { if x { <p/> } }</div>",
            "explicit stable id",
        ),
        (
            "<div>for x in xs { if x { <p id={x}/> } else { <p/> } }</div>",
            "explicit stable id",
        ),
        (
            "<div>for x in xs { \"text\" }</div>",
            "element wrapper with an explicit stable id",
        ),
        (
            "<div>for x in xs { {x} }</div>",
            "element wrapper with an explicit stable id",
        ),
    ];

    for (source, expected) in cases {
        let error = expand(source).expect_err(source).to_string();

        assert!(
            error.contains(expected),
            "{source}: expected {expected:?}, got {error:?}"
        );
    }
}

#[test]
fn empty_branches_and_nested_loops_expand_to_valid_rust() {
    let expanded = expand(
        r#"
        <div>
            if visible {} else if other {} else {}
            for outer in rows {
                if outer.enabled {
                    <div id={outer.id}>
                        for inner in outer.children {
                            <p id={inner.id}>{inner.name}</p>
                        }
                    </div>
                }
            }
        </div>
    "#,
    )
    .expect("valid control flow");

    syn::parse2::<syn::Expr>(expanded).expect("valid Rust expression");
}

#[test]
fn retained_views_and_handles_require_borrowed_owned_state() {
    for source in [
        "<view entity={&self.editor}/>",
        "<view entity={&editors[index]}/>",
        "<div focus={&self.focus} scroll={&self.scroll}><view entity={&self.editor}/></div>",
        "<div>for row in rows { <view entity={&row.editor}/> }</div>",
    ] {
        let expanded = expand(source).expect(source);

        syn::parse2::<syn::Expr>(expanded).expect("one expression");
    }

    for (source, expected) in [
        ("<view/>", "requires entity"),
        (
            "<view entity={&cx.new(|_| Editor::new())}/>",
            "construct it once outside render",
        ),
        (
            "<view entity={self.editor.clone()}/>",
            "must borrow stored state",
        ),
        (
            "<view entity={&self.editor} class=\"p-4\"/>",
            "accepts only entity",
        ),
        (
            "<view entity={&self.editor}>\"child\"</view>",
            "cannot have children",
        ),
        (
            "<div focus={&cx.focus_handle()}/>",
            "construct it once outside render",
        ),
        (
            "<div scroll={&ScrollHandle::new()}/>",
            "construct it once outside render",
        ),
    ] {
        assert!(
            expand(source)
                .expect_err(source)
                .to_string()
                .contains(expected),
            "{source}"
        );
    }
}

#[test]
fn keyboard_attributes_and_namespaced_actions_expand_to_typed_rust() {
    let expanded = expand(
        r#"
        <section key-context="Editor" focus={&self.focus} tab-navigation={true}
            on:Save={cx.listener(Self::save)}
            on:crate::actions::Cancel={cx.listener(Self::cancel)}>
            <button disabled={self.busy} tabindex={-1} focus={&self.save_focus}
                aria-keyshortcuts="Control+S" onclick={|_, _, _| {}}>"Save"</button>
            <div role={Role::Status}/>
        </section>
    "#,
    )
    .expect("keyboard syntax");

    syn::parse2::<syn::Expr>(expanded).expect("typed Rust expression");

    for (source, expected) in [
        (
            "<div disabled={true}/>",
            "disabled is supported only on button",
        ),
        (
            "<div on:Save={a} on:Save={b}/>",
            "duplicate attribute `on:Save`",
        ),
        ("<div onclick:Save={a}/>", "typed actions use on:Action"),
        ("<div on:={a}/>", "expected identifier"),
        ("<div on:Save-extra={a}/>", "expected `=`"),
        (
            "<img src=\"icon.png\" alt=\"Icon\" role={Role::Button}/>",
            "this attribute requires a container or button",
        ),
    ] {
        assert!(
            expand(source)
                .expect_err(source)
                .to_string()
                .contains(expected),
            "{source}"
        );
    }
}

#[test]
fn component_paths_properties_and_children_expand_to_struct_literals() {
    for source in [
        "<Badge id={id} label={label} highlighted={true}/>",
        "<ui::Card id={id} header={header} footer={None}>\"Body\"</ui::Card>",
        "<::ui::Card id={id}></::ui::Card>",
        "<crate::Panel children={children}/>",
        "<Panel>if visible { <Badge label={label}/> } for id in ids { <Badge id={id}/> }</Panel>",
    ] {
        let expanded = expand(source).expect(source);

        syn::parse2::<syn::Expr>(expanded).expect("one component expression");
    }

    for (source, expected) in [
        ("<ui::Card></Card>", "expected closing </ui::Card>"),
        ("<Card>", "missing closing </Card>"),
        ("<Card children={children}></Card>", "not both"),
        ("<Badge aria-label=\"Label\"/>", "Rust field names"),
        ("<Badge on:Save={handler}/>", "Rust field names"),
        (
            "<Panel>for id in ids { <Badge/> }</Panel>",
            "explicit stable id",
        ),
        ("<Card label={a} label={b}/>", "duplicate attribute `label`"),
    ] {
        let error = expand(source).expect_err(source).to_string();

        assert!(error.contains(expected), "{source}: {error}");
    }
}

#[test]
fn paragraphs_compose_inline_content_and_reject_layout_or_interactive_children() {
    for source in [
        "<p wrap={true} max-lines={3} ellipsis={true}>\"Hello \"<strong>{name}</strong><br/><em>\"Again\"</em></p>",
        "<p><span class=\"font-bold italic underline\" font-family=\"monospace\" style={highlight}>\"Text\"</span></p>",
        "<p>for (index, label) in labels { if index > 0 { \", \" } <span>{label}</span> }</p>",
        "<p>if let Some(value) = selected { <em>{value}</em> } else { \"None\" }</p>",
        "<p/>",
    ] {
        let expanded = expand(source).expect(source);

        syn::parse2::<syn::Expr>(expanded).expect("one paragraph expression");
    }

    for (source, expected) in [
        ("<p><div/></p>", "place layout elements"),
        ("<p><Card/></p>", "place layout elements"),
        (
            "<p><a href=\"https://example.com\">\"Link\"</a></p>",
            "place layout elements",
        ),
        ("<p><span id=\"inline\">\"Text\"</span></p>", "inline IDs"),
        (
            "<p><span class=\"text-xl\">\"Text\"</span></p>",
            "unsupported inline text utility",
        ),
        ("<p><span class={classes}/></p>", "literal text utility"),
        ("<p><br class=\"p-2\"/></p>", "br accepts no attributes"),
        ("<p><br>\"Text\"</br></p>", "br accepts no attributes"),
        ("<section wrap={true}/>", "unsupported attribute `wrap`"),
        ("<strong>\"Text\"</strong>", "unsupported view element"),
    ] {
        let error = expand(source).expect_err(source).to_string();

        assert!(error.contains(expected), "{source}: {error}");
    }
}

#[test]
fn expansion_snapshots() {
    let mut fixtures = vec![
        ("containers", r#"<section id="root" class="flex flex-col"><div/><span>{label}</span></section>"#.to_owned()),
        ("paragraph", r#"<p wrap={true} ellipsis={true} max-lines={2}>"Hi "<strong>{name}</strong><em>"!"</em><br/><span class="underline" style={highlight}>"More"</span></p>"#.to_owned()),
        ("headings", r#"<div><h1>"Title"</h1><h2>"Count: "{count.to_string()}</h2><h3/><h4/><h5/><h6/></div>"#.to_owned()),
        ("button", r#"<button disabled={busy} focus={&self.focus} tabindex={-1} onclick={handler} style={style}>"Save"</button>"#.to_owned()),
        ("image", r#"<img id="preview" src={path} alt="Preview" style={style}/>"#.to_owned()),
        ("entity", r#"<view entity={&self.editor}/>"#.to_owned()),
        ("component", r#"<ui::Badge id={id} label={label} highlighted={true}/>"#.to_owned()),
        ("slots", r#"<Card id={id} header={header} footer={None}><p>"Body"</p>{widget}</Card>"#.to_owned()),
        ("branches", r#"<div>if let Some(value) = selected { <p>{value}</p> } else if busy { "Loading" } else { <span/> }</div>"#.to_owned()),
        ("loops", r#"<ul>for row in rows { if row.visible { <li id={row.id}>{row.label}</li> } }</ul>"#.to_owned()),
        ("actions", r#"<section key-context="Editor" on:Save={save} on:crate::actions::Cancel={cancel}><button onclick={click}>"Save"</button></section>"#.to_owned()),
        ("conditional_classes", r#"<div class="relative" class:shrink-0={fixed} class:font-bold={selected}/>"#.to_owned()),
        ("navigation", r#"<section focus={&self.focus} scroll={&self.scroll} tab-navigation={true}><button tabindex={0}/></section>"#.to_owned()),
    ];

    let mut utilities = "flex block grid hidden flex-col flex-row flex-wrap flex-1 items-start items-center items-end justify-start justify-center justify-end justify-between w-full h-full size-full min-w-0 min-h-0 overflow-hidden overflow-scroll overflow-x-scroll overflow-y-scroll rounded rounded-none rounded-sm rounded-md rounded-lg rounded-xl rounded-full border border-0 border-1 border-2 shadow-sm shadow-md shadow-lg cursor-pointer text-xs text-sm text-base text-lg text-xl text-2xl text-3xl text-left text-center text-right font-normal font-medium font-semibold font-bold".to_owned();

    for columns in 1..=12 {
        utilities.push_str(&format!(" grid-cols-{columns}"));
    }

    for prefix in [
        "p", "px", "py", "pt", "pb", "pl", "pr", "m", "mx", "my", "mt", "mb", "ml", "mr", "gap",
    ] {
        for scale in [0, 1, 2, 3, 4, 5, 6, 7, 8, 12, 16, 20, 24, 32] {
            utilities.push_str(&format!(" {prefix}-{scale}"));
        }
    }

    fixtures.push(("utilities", format!("<div class={utilities:?}/>")));
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/view/snapshots");

    for (name, source) in fixtures {
        let expanded = expand(&source).expect(name).to_string() + "\n";
        let path = directory.join(format!("{name}.txt"));

        if std::env::var_os("RAKE_UPDATE_SNAPSHOTS").is_some() {
            std::fs::write(&path, &expanded).expect("write snapshot");
        }

        assert_eq!(
            std::fs::read_to_string(path).expect("snapshot exists"),
            expanded,
            "snapshot {name}; review changes before running RAKE_UPDATE_SNAPSHOTS=1"
        );
    }
}

#[test]
fn collects_independent_errors_and_recovers_attribute_boundaries() {
    let error = expand(r#"<div class=42 title="invalid"><img src="icon.png"/></div>"#).unwrap_err();

    let errors = error
        .into_iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>();

    assert_eq!(errors.len(), 3);
    assert!(errors[0].contains("quoted strings"));
    assert!(errors[1].contains("unsupported attribute `title`"));
    assert!(errors[2].contains("img requires alt"));
}

#[test]
fn documented_utilities_match_the_checked_table() {
    use super::classes::{FIXED, PREFIXES, SCALES};

    let docs = include_str!("../../../../docs/experimental-view.md");

    let list = docs
        .split("<!-- view-utilities:start -->")
        .nth(1)
        .unwrap()
        .split("<!-- view-utilities:end -->")
        .next()
        .unwrap();

    let names = list
        .split('`')
        .enumerate()
        .filter_map(|(index, value)| (index % 2 == 1).then_some(value))
        .collect::<Vec<_>>();

    for (utility, _) in FIXED {
        assert!(
            names.contains(utility),
            "missing documented utility {utility}"
        );
    }

    for prefix in PREFIXES {
        assert!(names.contains(&format!("{prefix}-N").as_str()));
    }

    for name in names {
        let utilities = if name == "grid-cols-N" {
            (1..=12)
                .map(|n| format!("grid-cols-{n}"))
                .collect::<Vec<_>>()
        } else if let Some(prefix) = name.strip_suffix("-N") {
            SCALES.iter().map(|n| format!("{prefix}-{n}")).collect()
        } else {
            vec![name.to_owned()]
        };

        for utility in utilities {
            expand(&format!("<div class={utility:?}/>")).expect(&utility);
        }
    }
}
