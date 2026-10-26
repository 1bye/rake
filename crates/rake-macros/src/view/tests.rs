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
