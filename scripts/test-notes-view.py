#!/usr/bin/env python3
"""Check the Fieldnotes workspace and retained editors in a live inspection capture."""
import argparse
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--session", type=Path)
    source.add_argument("--snapshot", type=Path)
    parser.add_argument("--instance", default="dev.rake.fieldnotes")
    args = parser.parse_args()

    if args.snapshot:
        capture = json.loads(args.snapshot.read_text())
    else:
        capture = json.loads(subprocess.check_output(
            [str(ROOT / "target/debug/rake"), "ctl", "--instance", args.instance, "--json", "inspect", "tree"],
            env={**os.environ, "RAKE_CONTROL_DIR": str(args.session / "control")}, timeout=15,
        ))

    snapshot = capture["value"]
    assert snapshot["status"] == "captured", snapshot["status"]
    nodes = {node["semantic_id"]: node for node in snapshot["elements"] if node.get("semantic_id")}
    workspace = nodes["notes.workspace"]
    rows = [node for name, node in nodes.items() if name.startswith("notes.note.")]
    assert rows, "Use a test notebook with visible notes"
    for row in rows:
        assert row["parent"] == workspace["id"], row["semantic_id"]
        assert row["accessibility"]["role"] == "Button", row["semantic_id"]
        assert int(row["semantic_id"].rsplit(".", 1)[1]) > 0, row["semantic_id"]

    editors = {}
    for name in ("notes.search", "notes.title", "notes.body"):
        mount = nodes[name]
        assert mount["parent"] == workspace["id"], name
        assert mount["layout"]["width"] > 0 and mount["layout"]["height"] > 0, name
        assert not mount["accessibility"]["hidden"], name
        assert mount["navigation"]["focus"], name
        assert mount.get("text") is None and not mount["text_editable"], name
        assert "value" not in mount["accessibility"], name
        editors[name] = mount["layout"]

    assert editors["notes.search"]["x"] < editors["notes.title"]["x"]
    assert editors["notes.title"]["y"] < editors["notes.body"]["y"]
    print(f"Fieldnotes workspace verified: {len(snapshot['elements'])} nodes, {len(rows)} keyed rows, 3 private focus targets")


if __name__ == "__main__":
    main()
