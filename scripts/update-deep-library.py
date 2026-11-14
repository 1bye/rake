#!/usr/bin/env python3
"""Bundle the Dylint crate for installed CLI binaries; timestamps are deterministic."""
from pathlib import Path
import zipfile

root = Path(__file__).resolve().parents[1]
library = root / "crates/rake-cli/lints/rake-gpui"
output = root / "crates/rake-cli/src/lint/deep-library.zip"
with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
    for name in ["Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml", "src/lib.rs"]:
        info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
        info.compress_type = zipfile.ZIP_DEFLATED
        info.external_attr = 0o100644 << 16
        archive.writestr(info, (library / name).read_bytes())
print(f"Updated {output.relative_to(root)}")
