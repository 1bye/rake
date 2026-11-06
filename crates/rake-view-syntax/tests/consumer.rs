//! Covers: public syntax-tree and lowering imports from an external consumer.
//! Real: syn parser and shared lowering; no GPUI linkage.
//! Doubles: none.
//! Requires: no display or GPU.

use rake_view_syntax::{AttributeValue, Node, View};

#[test]
fn consumers_can_inspect_the_tree_and_lower_the_same_view() {
    let view: View = syn::parse_str("<button id=\"save\">Save</button>").unwrap();
    let root = view.root();

    assert!(root.tag.is_ident("button"));
    assert_eq!(root.attributes[0].name, "id");
    assert!(matches!(
        root.attributes[0].value,
        AttributeValue::Expression(_)
    ));
    assert!(matches!(&root.children[0], Node::Text(text) if text.value() == "Save"));
    let output = view.expand_resilient(&quote::quote!(::renamed)).unwrap();

    syn::parse2::<syn::Expr>(output).expect("lowered view remains one Rust expression");
}

#[test]
fn source_layout_retains_delimiters_and_rejects_recovered_syntax_errors() {
    use rake_view_syntax::LayoutKind;

    let source = "<div id={value}>if ready {<span/>} else {}</div>";
    let view: View = syn::parse_str(source).unwrap();
    let layout = view.layout().unwrap();

    assert_eq!(layout.kind, LayoutKind::Element);
    assert_eq!(layout.start.start().column, 0);
    assert_eq!(layout.end.end().column, source.len());
    assert_eq!(layout.children[0].kind, LayoutKind::Opening);
    assert_eq!(layout.children[0].children[1].kind, LayoutKind::Attribute);
    assert_eq!(layout.children[1].kind, LayoutKind::Flow);
    assert_eq!(layout.children[1].children[1].kind, LayoutKind::Block);
    assert_eq!(layout.children[2].kind, LayoutKind::Closing);

    let recovered: View = syn::parse_str("<div id=123/>").unwrap();
    assert!(recovered.layout().is_err());
}
