#!/usr/bin/env python3
"""Covers: Fieldnotes' authored inspection hierarchy, keyed note rows and editor mounts.
Real: captured GPUI tree through the running runtime/CLI. Doubles: none.
Requires: a visible, drawn rake-notes devtools session, or --snapshot for a saved capture.
"""
import argparse
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
source = parser.add_mutually_exclusive_group(required=True)
source.add_argument("--session", type=Path)
source.add_argument("--snapshot", type=Path)
parser.add_argument("--instance", default="dev.rake.fieldnotes")
args = parser.parse_args()
if args.snapshot:
    capture = json.loads(args.snapshot.read_text())
else:
    import os
    cli = Path(os.environ.get("CARGO_TARGET_DIR", str(Path(__file__).resolve().parents[1] / "target"))) / "debug/rake"
    capture = json.loads(subprocess.check_output([str(cli), "ctl", "--instance", args.instance,
        "--json", "inspect", "tree"], env={**os.environ, "RAKE_CONTROL_DIR": str(args.session / "control")}, timeout=15))

snapshot = capture["value"]
assert snapshot["status"] == "captured", snapshot["status"]
nodes = {node["semantic_id"]: node for node in snapshot["elements"] if node.get("semantic_id")}
if args.session and "notes.sidebar" not in nodes:
    runtime = json.loads(subprocess.check_output([str(cli), "ctl", "--instance", args.instance,
        "--json", "snapshot"], env={**os.environ, "RAKE_CONTROL_DIR": str(args.session / "control")}, timeout=15))["value"]
    if runtime["work"]["host_visibility"] == "Hidden":
        raise SystemExit("Fieldnotes is hidden and has not drawn its page yet; bring its browser tab/window forward before checking the tree")
for child, parent, tag in [
    ("notes.sidebar", "notes.workspace", "aside"),
    ("notes.collections", "notes.sidebar", "nav"),
    ("notes.all", "notes.collections", "button"),
    ("notes.list", "notes.workspace", "section"),
    ("notes.entries", "notes.list", "div"),
    ("notes.document", "notes.workspace", "main"),
    ("notes.document-header", "notes.document", "header"),
    ("notes.document-content", "notes.document", "article"),
    ("notes.title", "notes.document-content", "div"),
    ("notes.body", "notes.document-content", "div"),
]:
    assert child in nodes, f"Missing authored container: {child}"
    assert nodes[child]["parent"] == nodes[parent]["id"], f"{child} must be nested under {parent}"
    assert nodes[child].get("view", {}).get("tag") == tag, f"{child} must retain its authored {tag} tag"
for node in nodes.values():
    if node["semantic_id"].startswith("notes.note."):
        assert node["parent"] == nodes["notes.entries"]["id"], "Note rows must belong to the list"
        assert node.get("view", {}).get("tag") == "button"
        assert node["accessibility"]["role"] == "Button", node["semantic_id"]
        assert int(node["semantic_id"].rsplit(".", 1)[1]) > 0, node["semantic_id"]
rows = [node for name, node in nodes.items() if name.startswith("notes.note.")]
assert rows, "Use a test notebook with visible notes"
for editor in ["notes.title", "notes.body", "notes.search"]:
    mount = nodes[editor]
    assert mount["layout"]["width"] > 0 and mount["layout"]["height"] > 0, editor
    assert mount["navigation"]["focus"], editor
    assert not mount["accessibility"]["hidden"], editor
    assert mount.get("text") is None and not mount["text_editable"], editor
    assert "value" not in mount["accessibility"], editor
assert nodes["notes.search"]["layout"]["x"] < nodes["notes.title"]["layout"]["x"]
assert nodes["notes.title"]["layout"]["y"] < nodes["notes.body"]["layout"]["y"]
# Shared Input/Textarea mount as opaque GPUI children; their retained state and
# accessibility adapters replace the snapshot's custom Editor entity wrapper.
assert any(node.get("view", {}).get("tag") == "Text" for node in snapshot["elements"]), "Authored labels must have text children"
print(f"Fieldnotes authored hierarchy verified: {len(snapshot['elements'])} captured nodes")
