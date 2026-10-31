// Real Chrome file/clipboard/Worker/ServiceWorker host boundaries using production adapters.
// No GPUI rendering/accessibility claim. Requires Chrome, Node 22+, Python 3.11+.
import assert from 'node:assert/strict';
import {spawn, spawnSync} from 'node:child_process';
import {createServer} from 'node:http';
import {mkdtemp, readFile, writeFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {setTimeout as delay} from 'node:timers/promises';

const root = fileURLToPath(new URL('../', import.meta.url));
const temporary = await mkdtemp(join(tmpdir(), 'rake-web-host-'));
const chrome = process.env.CHROME_BIN ?? (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : 'google-chrome');
const environment = {...process.env, CARGO_PROFILE_DEV_DEBUG: '0', CARGO_INCREMENTAL: '0'};
const built = spawnSync('cargo', ['build', '-p', 'rake-web-host-fixture', '--lib', '--target', 'wasm32-unknown-unknown', '--locked'], {cwd: root, env: environment, stdio: 'inherit'});
assert.equal(built.status, 0, 'build the actual Rust browser fixture');
const bindgen = process.env.WASM_BINDGEN ?? 'wasm-bindgen';
const expected = spawnSync('python3', ['-c', 'import tomllib; print(next(p["version"] for p in tomllib.load(open("Cargo.lock", "rb"))["package"] if p["name"] == "wasm-bindgen"))'], {cwd: root, encoding: 'utf8'});
const tooling = spawnSync(bindgen, ['--version'], {encoding: 'utf8'});
assert.equal(tooling.stdout?.trim(), `wasm-bindgen ${expected.stdout.trim()}`, 'install the matching wasm-bindgen-cli or set WASM_BINDGEN');
const bindings = spawnSync(bindgen, ['--target', 'web', '--out-dir', join(temporary, 'bindings'), join(root, 'target/wasm32-unknown-unknown/debug/rake_web_host_fixture.wasm')], {stdio: 'inherit'});
assert.equal(bindings.status, 0, 'generate fixture bindings');
const bootstrap = `
import sys, json, shutil
from pathlib import Path
from types import SimpleNamespace
sys.path.insert(0, str(Path(sys.argv[1]) / 'scripts'))
from bundle import BundleConfig, web_deployment
root, work = Path(sys.argv[1]), Path(sys.argv[2])
for label in ['old', 'new']:
    site = work / label
    site.mkdir()
    shutil.copytree(work / 'bindings', site / 'pkg')
    (site / 'platform.js').write_text((root / 'crates/rake/src/platform/browser.js').read_text())
    (site / 'runtime.js').write_text((root / 'crates/rake/src/web/browser.js').read_text())
    (site / 'count.worker.js').write_text('self.onmessage=e=>self.postMessage(JSON.stringify({length:JSON.parse(e.data).text.length,build:' + json.dumps(label) + '}));')
    (site / 'never.worker.js').write_text('self.onmessage=()=>{};')
    (site / 'index.html').write_text('<html><head></head><body><button id="pick">Pick file</button><script type="module">import * as platform from "./platform.js"; import * as runtime from "./runtime.js"; import * as fixture from "./pkg/rake_web_host_fixture.js"; await fixture.default();fixture.start();window.fixture=fixture;window.platform=platform;window.runtime=runtime;window.workersStarted=0;window.workersTerminated=0;window.Worker=class extends window.Worker {constructor(...args){super(...args);window.workersStarted++;}terminate(){window.workersTerminated++;super.terminate();}};window.build=' + json.dumps(label) + ';document.querySelector("#pick").onclick=async()=>{try{const f=await platform.pick_file();window.picked=f?{name:f.name,text:new TextDecoder().decode(await platform.read_file(f,100))}:null;}catch(e){window.pickError=e.message;}};window.ready=true;</script></body></html>')
    package = {'name':'host-fixture','version':'1.0.0','metadata':{'rake':{'bundle':{'identifier':'dev.rake.host-test','web':{'base_path':'/tools/','offline':True}}}}}
    web_deployment(SimpleNamespace(package=package,bundle=BundleConfig(package,site),args=SimpleNamespace(base_path=None,routing=None)),site)
`;
const generated = spawnSync('python3', ['-c', bootstrap, root, temporary], {encoding: 'utf8'});
assert.equal(generated.status, 0, generated.stderr);
let current = 'old';
let serverOffline = false;
const server = createServer(async (request, response) => {
  if (serverOffline) { request.socket.destroy(); return; }
  try {
    const url = new URL(request.url, 'http://localhost');
    const relative = decodeURIComponent(url.pathname).replace(/^\/tools\//, '');
    if (!url.pathname.startsWith('/tools/') || relative.split('/').some(part => part === '..')) { response.writeHead(404).end(); return; }
    let data;
    let path;
    for (const build of [current, current === 'old' ? 'new' : 'old']) {
      try { path = join(temporary, build, relative || 'index.html'); data = await readFile(path); break; } catch (_) { /* Immutable old build fallback. */ }
    }
    if (!data && !relative.includes('.')) { path = join(temporary, current, 'index.html'); data = await readFile(path); }
    if (!data) { response.writeHead(404).end(); return; }
    response.writeHead(200, {'Content-Type': path.endsWith('.js') ? 'text/javascript' : path.endsWith('.wasm') ? 'application/wasm' : path.endsWith('.json') ? 'application/json' : 'text/html', 'Cache-Control': 'no-store'}).end(data);
  } catch (error) { response.writeHead(500).end(String(error)); }
});
await new Promise((resolve, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve); });
const origin = `http://127.0.0.1:${server.address().port}`;
const profile = join(temporary, 'profile');
let stderr = '';
const processChrome = spawn(chrome, ['--headless=new', '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank'], {stdio: ['ignore', 'ignore', 'pipe']});
processChrome.stderr.on('data', data => { stderr = (stderr + data).slice(-8000); });
let socket;
const pending = new Map();
const events = [];
let sequence = 0;
async function eventually(check, timeout = 20_000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const value = await check();
    if (value) return value;
    if (processChrome.exitCode !== null) throw new Error(`Chrome exited (${processChrome.exitCode}): ${stderr}`);
    await delay(50);
  }
  throw new Error('Browser host check timed out');
}
function call(method, params = {}, sessionId) {
  return new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(new Error(`${method} timed out`)); }, 25_000);
    pending.set(id, {resolve, reject, timer});
    socket.send(JSON.stringify({id, method, params, sessionId}));
  });
}
async function evaluate(session, expression, userGesture = false) {
  const reply = await call('Runtime.evaluate', {expression, returnByValue: true, awaitPromise: true, userGesture}, session);
  if (reply.exceptionDetails) throw new Error(JSON.stringify(reply.exceptionDetails));
  return reply.result.value;
}
async function page(url) {
  const {targetId} = await call('Target.createTarget', {url: 'about:blank'});
  const {sessionId} = await call('Target.attachToTarget', {targetId, flatten: true});
  await call('Page.enable', {}, sessionId);
  await call('Network.enable', {}, sessionId);
  await call('Page.navigate', {url}, sessionId);
  await eventually(() => evaluate(sessionId, 'window.ready === true'));
  return sessionId;
}
try {
  const port = await eventually(async () => { try { return (await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; } catch (_) { return null; } });
  const browser = await (await fetch(`http://127.0.0.1:${port}/json/version`)).json();
  socket = new WebSocket(browser.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, {once: true}); socket.addEventListener('error', reject, {once: true}); });
  socket.addEventListener('message', event => {
    const reply = JSON.parse(event.data);
    const request = pending.get(reply.id);
    if (request) {
      pending.delete(reply.id); clearTimeout(request.timer);
      reply.error ? request.reject(new Error(JSON.stringify(reply.error))) : request.resolve(reply.result);
    } else if (reply.method) { events.push(reply); if (events.length > 1000) events.shift(); }
  });
  const old = await page(origin + '/tools/');
  assert.equal(await evaluate(old, 'window.build'), 'old');
  await eventually(() => evaluate(old, 'document.documentElement.dataset.rakeOffline === "ready"'));
  await call('Page.reload', {}, old);
  await eventually(() => evaluate(old, 'window.ready === true && !!navigator.serviceWorker.controller'));
  assert.deepEqual(await evaluate(old, 'runtime.worker_promise(runtime.start_worker("count.worker.js", JSON.stringify({text:"hello"}), 5000)).then(JSON.parse)'), {length: 5, build: 'old'});
  assert.equal(await evaluate(old, 'fixture.cancel_worker(); window.workersStarted === window.workersTerminated'), true, 'dropping an unpolled Rust worker request terminates its actual Worker');
  await call('Page.setInterceptFileChooserDialog', {enabled: true}, old);
  const textFile = join(temporary, 'hello.txt');
  await writeFile(textFile, 'Unicode 漢字');
  await evaluate(old, 'document.querySelector("#pick").click()', true);
  const chooser = await eventually(() => events.find(event => event.method === 'Page.fileChooserOpened' && event.sessionId === old));
  await call('DOM.setFileInputFiles', {files: [textFile], backendNodeId: chooser.params.backendNodeId}, old);
  await eventually(() => evaluate(old, '!!window.picked || !!window.pickError'));
  assert.deepEqual(await evaluate(old, 'window.picked'), {name: 'hello.txt', text: 'Unicode 漢字'});
  await call('Browser.grantPermissions', {origin, permissions: ['clipboardReadWrite', 'clipboardSanitizedWrite']});
  // Keep existing clipboard content inside this page and restore it without logging it.
  await evaluate(old, 'navigator.clipboard.readText().then(value => { window.previousClipboard = value; })', true);
  try {
    await evaluate(old, 'platform.write_clipboard("Rake host boundary fixture")', true);
    assert.equal(await evaluate(old, 'platform.read_clipboard()', true), 'Rake host boundary fixture');
  } finally { await evaluate(old, 'navigator.clipboard.writeText(window.previousClipboard)', true); }
  await evaluate(old, 'platform.preference_set("test", "theme", "dark")');
  assert.equal(await evaluate(old, 'platform.preference_get("test", "theme")'), 'dark');
  current = 'new';
  const version = await evaluate(old, 'runtime.check_deployment("/tools/").then(JSON.parse)');
  assert.notEqual(version.build, await evaluate(old, 'JSON.parse(runtime.deployment_metadata()).build'));
  assert.equal(await evaluate(old, 'fixture.reload_update()'), 'blocked', 'the actual Rust quit guard must veto deployment reload');
  assert.equal(await evaluate(old, 'window.build'), 'old');
  await evaluate(old, 'navigator.serviceWorker.getRegistration().then(registration => registration.update())');
  await eventually(() => evaluate(old, 'navigator.serviceWorker.getRegistration().then(registration => !!registration.waiting)'));
  assert.equal(await evaluate(old, 'window.build'), 'old', 'installing an update must not reload a live tab');
  const next = await page(origin + '/tools/');
  assert.equal(await evaluate(next, 'window.build'), 'new');
  assert.equal(await evaluate(old, 'window.build'), 'old');
  assert.deepEqual(await evaluate(old, 'runtime.worker_promise(runtime.start_worker("count.worker.js", JSON.stringify({text:"retained"}), 5000)).then(JSON.parse)'), {length: 8, build: 'old'});
  // Page-level CDP offline emulation alone does not disable a worker's network.
  // Refuse transport at the server too, so the real service worker must use cache.
  serverOffline = true;
  await call('Network.emulateNetworkConditions', {offline: true, latency: 0, downloadThroughput: 0, uploadThroughput: 0}, old);
  await call('Page.navigate', {url: origin + '/tools/offline/deep-link'}, old);
  await eventually(() => evaluate(old, 'window.ready === true && window.build === "old"'));
  assert.equal(await evaluate(old, 'platform.preference_get("test", "theme")'), 'dark');
  assert.equal(await evaluate(old, 'runtime.check_deployment("/tools/").then(() => false, () => true)'), true, 'version endpoint must remain network-only');
  assert.deepEqual(await evaluate(old, 'runtime.worker_promise(runtime.start_worker("count.worker.js", JSON.stringify({text:"offline"}), 5000)).then(JSON.parse)'), {length: 7, build: 'old'});
  serverOffline = false;
  await call('Network.emulateNetworkConditions', {offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1}, old);
  await evaluate(old, 'fixture.allow_quit(true); fixture.reload_update(); true');
  await eventually(() => evaluate(old, 'window.ready === true && window.build === "new"'));
  console.log(JSON.stringify({browser: browser.Browser, passed: ['actual file selection and Unicode read', 'clipboard with automation permission and content restoration', 'preferences', 'module workers and Rust drop cancellation', 'waiting update preserves open tabs', 'old/new immutable assets coexist', 'offline deep-link fallback', 'network-only version checks', 'Rust quit guard blocks reload and approved quit reloads']}));
} finally {
  for (const request of pending.values()) { clearTimeout(request.timer); request.reject(new Error('browser check ended')); }
  socket?.close();
  processChrome.kill('SIGTERM');
  await Promise.race([new Promise(resolve => processChrome.once('exit', resolve)), delay(5000)]);
  if (processChrome.exitCode === null) processChrome.kill('SIGKILL');
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
  await rm(temporary, {recursive: true, force: true});
}
