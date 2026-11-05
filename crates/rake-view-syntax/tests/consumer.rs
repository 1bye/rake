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
