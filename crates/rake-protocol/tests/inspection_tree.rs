//! Covers: escaped subtree copying, paths, invalid graphs and output bounds.
//! Real: portable public inspection contracts. Doubles: none. Requires: none.

use rake_protocol::inspection::{Element, MAX_SNAPSHOT_BYTES, tree};
use serde_json::json;

fn nodes() -> Vec<Element> {
    serde_json::from_value(json!([
        {"id":"p","kind":"p","semantic_id":"a\"<&","metadata_truncated":false},
        {"id":"text","parent":"p","kind":"Text","text":"<script>世界</script>","metadata_truncated":false}
    ])).unwrap()
}

#[test]
fn copy_preserves_text_and_escapes_synthetic_markup() {
    let nodes = nodes();

    assert_eq!(
        tree::text(&nodes, "p").as_deref(),
        Some("<script>世界</script>")
    );
    assert_eq!(
        tree::markup(&nodes, "p").as_deref(),
        Some("<p id=\"a&quot;&lt;&amp;\">&lt;script&gt;世界&lt;/script&gt;</p>")
    );
    assert_eq!(
        tree::path(&nodes, "text").as_deref(),
        Some("/p[1][@id=\"a\\\"<&\"]/Text[1]")
    );
    assert_eq!(tree::subtree(&nodes, "p").unwrap().len(), 2);
    assert!(tree::text(&nodes, "missing").is_none());
}

#[test]
fn invalid_graphs_and_oversized_payloads_fail_without_partial_copies() {
    let mut nodes = nodes();

    nodes[0].parent = Some("text".into());
    assert!(tree::markup(&nodes, "p").is_none());
    assert!(tree::path(&nodes, "text").is_none());
    nodes[0].parent = None;
    nodes[1].text = Some("x".repeat(MAX_SNAPSHOT_BYTES + 1));
    assert!(tree::text(&nodes, "p").is_none());
    assert!(tree::markup(&nodes, "p").is_none());
    nodes[1].text = Some("\u{0001}".repeat(MAX_SNAPSHOT_BYTES / 4));
    assert!(
        tree::markup(&nodes, "p").is_none(),
        "bound JSON escaping, not only raw output"
    );
    nodes.push(nodes[0].clone());
    assert!(tree::subtree(&nodes, "p").is_none());
}
