#!/usr/bin/env python3
"""Compare the shared parser with an origin/main snapshot on the same inspector tokens.

Run with CARGO_TARGET_DIR=<worktree>/target CARGO_BUILD_JOBS=4. All generated
sources and the standalone benchmark lockfile stay under that target directory.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tomllib

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--baseline", default="origin/main")
parser.add_argument("--fixture-ref", default="HEAD")
parser.add_argument("--iterations", type=int, default=200)
parser.add_argument("--samples", type=int, default=11)
args = parser.parse_args()
root = Path(__file__).resolve().parent.parent
target = Path(os.environ["CARGO_TARGET_DIR"]).expanduser().resolve()
if target != root / "target" or os.environ.get("CARGO_BUILD_JOBS") != "4":
    raise SystemExit("use this worktree's target directory and CARGO_BUILD_JOBS=4")
bench = target / "issue31-review" / "parser-bench"
baseline = bench / "main-parser"
baseline.mkdir(parents=True, exist_ok=True)

def git(*arguments):
    return subprocess.check_output(["git", "-C", str(root), *arguments], text=True)

revision = git("rev-parse", args.baseline).strip()
for name in git("ls-tree", "-r", "--name-only", revision, "crates/rake-view-syntax/src").splitlines():
    path = baseline / Path(name).relative_to("crates/rake-view-syntax")
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(git("show", f"{revision}:{name}"))

lock = tomllib.loads((root / "Cargo.lock").read_text())
versions = {package["name"]: package["version"] for package in lock["package"]
            if package["name"] != "syn" or package["version"].startswith("2.")}
deps = "\n".join([
    f'proc-macro2 = {{ version = "={versions["proc-macro2"]}", features = ["span-locations"] }}',
    f'quote = "={versions["quote"]}"',
    f'syn = {{ version = "={versions["syn"]}", features = ["full", "visit"] }}',
])
baseline.joinpath("Cargo.toml").write_text(
    '[package]\nname = "rake-view-syntax-baseline"\nversion = "0.0.0"\nedition = "2024"\n'
    '\n[dependencies]\n' + deps + '\n'
)
bench.joinpath("Cargo.toml").write_text(
    '[package]\nname = "rake-parser-timing"\nversion = "0.0.0"\nedition = "2024"\n'
    '\n[workspace]\n\n[dependencies]\n' + deps
    + '\nbaseline = { package = "rake-view-syntax-baseline", path = "main-parser" }\n'
    + '\ncurrent = { package = "rake-view-syntax", path = '
    + json.dumps(str(root / "crates/rake-view-syntax")) + ' }\n'
)
fixture = bench / "inspector.rs"
fixture.write_text(git("show", f"{args.fixture_ref}:examples/inspector/src/view.rs"))
source = r'''
use std::{hint::black_box, time::Instant};
use proc_macro2::TokenStream;
use syn::{visit::Visit};

fn main() {
    let arguments: Vec<_> = std::env::args().collect();
    let iterations: usize = arguments[2].parse().unwrap();
    let samples: usize = arguments[3].parse().unwrap();
    let source = std::fs::read_to_string(&arguments[1]).unwrap();
    struct Corpus(Vec<TokenStream>);
    impl<'ast> Visit<'ast> for Corpus {
        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            if mac.path.segments.last().is_some_and(|segment| segment.ident == "view") {
                self.0.push(mac.tokens.clone());
            }
        }
    }
    let mut corpus = Corpus(Vec::new());
    corpus.visit_file(&syn::parse_file(&source).unwrap());
    assert!(!corpus.0.is_empty());
    let modes: &[(&str, fn(TokenStream))] = &[
        ("origin_main", |tokens| { black_box(syn::parse2::<baseline::View>(tokens).unwrap()); }),
        ("current_default", |tokens| { black_box(syn::parse2::<current::View>(tokens).unwrap()); }),
    ];
    let mut timings = vec![Vec::new(); modes.len()];
    for (_, parse) in modes {
        for _ in 0..10 { for tokens in &corpus.0 { parse(tokens.clone()); } }
    }
    for sample in 0..samples {
        for step in 0..modes.len() {
            let index = if sample % 2 == 0 { step } else { modes.len() - step - 1 };
            let started = Instant::now();
            for _ in 0..iterations {
                for tokens in &corpus.0 { modes[index].1(tokens.clone()); }
            }
            timings[index].push(started.elapsed().as_secs_f64() * 1_000_000.0 / iterations as f64);
        }
    }
    let mut baseline = 0.0;
    println!("fixture_invocations={},iterations={},samples={}", corpus.0.len(), iterations, samples);
    println!("mode,median_us_per_corpus,relative_percent");
    for ((name, _), times) in modes.iter().zip(&mut timings) {
        times.sort_by(f64::total_cmp);
        let median = times[times.len() / 2];
        if baseline == 0.0 { baseline = median; }
        println!("{name},{median:.3},{:.2}", (median / baseline - 1.0) * 100.0);
    }
}
'''
bench.joinpath("src").mkdir(exist_ok=True)
bench.joinpath("src/main.rs").write_text(source)
print(f"baseline={revision}, fixture_ref={args.fixture_ref}, release profile; lexing excluded", flush=True)
subprocess.run([
    "cargo", "run", "--release", "--offline", "--manifest-path", str(bench / "Cargo.toml"),
    "--bin", "rake-parser-timing", "--", str(fixture), str(args.iterations), str(args.samples)
], cwd=root, check=True)
