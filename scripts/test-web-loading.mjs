// Covers the real Fieldnotes Wasm module graph and its startup-error display.
// Requires Rust's Web target, matching wasm-bindgen-cli, Node 22+ and Python 3.11+.
// Module linking runs in Node; GPU rendering is verified separately in a browser.
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtemp, readFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const temporary = await mkdtemp(join(tmpdir(), 'rake-web-loading-'));
try {
  let bindings = process.argv[2];
  if (!bindings) {
    const environment = {...process.env, CARGO_PROFILE_DEV_DEBUG: '0', CARGO_INCREMENTAL: '0'};
    const build = spawnSync('cargo', ['build', '-p', 'rake-notes', '--lib', '--features', 'rake/devtools', '--target', 'wasm32-unknown-unknown', '--locked'], {cwd: root, env: environment, stdio: 'inherit'});
    assert.equal(build.status, 0, 'build Fieldnotes with its actual GPUI dependencies');
    const bindgen = process.env.WASM_BINDGEN ?? 'wasm-bindgen';
    const expected = spawnSync('python3', ['-c', 'import tomllib; print(next(p["version"] for p in tomllib.load(open("Cargo.lock", "rb"))["package"] if p["name"] == "wasm-bindgen"))'], {cwd: root, encoding: 'utf8'});
    assert.equal(expected.status, 0, 'read the locked wasm-bindgen version');
    const tooling = spawnSync(bindgen, ['--version'], {encoding: 'utf8'});
    assert.equal(tooling.stdout?.trim(), `wasm-bindgen ${expected.stdout.trim()}`, 'install matching wasm-bindgen-cli or set WASM_BINDGEN');
    const target = resolve(root, process.env.CARGO_TARGET_DIR ?? 'target');
    const generated = spawnSync(bindgen, ['--target', 'web', '--out-dir', temporary, join(target, 'wasm32-unknown-unknown/debug/rake_notes.wasm')], {stdio: 'inherit'});
    assert.equal(generated.status, 0, 'generate browser bindings');
    bindings = join(temporary, 'rake_notes.js');
  }
  const module = await import(pathToFileURL(resolve(bindings)).href);
  assert.equal(typeof module.default, 'function', 'the complete generated module graph must resolve before startup');

  const html = await readFile(join(root, 'examples/notes/web/index.html'), 'utf8');
  const script = html.match(/<script type="module">([\s\S]*?)<\/script>/)[1];
  const failure = 'data:text/javascript,throw new Error("module loading fixture")';
  const fixture = script.replace('./pkg/rake_notes.js', failure);
  assert.notEqual(fixture, script, 'inject the failure at the actual bindings import');
  const display = {hidden: true, textContent: ''};
  const document = globalThis.document;
  const error = console.error;
  try {
    globalThis.document = {querySelector: () => display};
    console.error = () => {};
    await import(`data:text/javascript,${encodeURIComponent(fixture)}`);
  } finally {
    if (document === undefined) delete globalThis.document;
    else globalThis.document = document;
    console.error = error;
  }
  assert.equal(display.hidden, false, 'a module-loading failure must show the error panel');
  assert.match(display.textContent, /Fieldnotes could not start: Error: module loading fixture/);
  console.log('PASS: Fieldnotes bindings resolve and module-loading failures produce a visible startup error');
} finally {
  await rm(temporary, {recursive: true, force: true});
}
