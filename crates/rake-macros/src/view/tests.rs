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
