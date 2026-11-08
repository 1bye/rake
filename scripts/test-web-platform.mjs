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
import {createBrowserCdp} from './browser-test-cdp.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const temporary = await mkdtemp(join(tmpdir(), 'rake-web-host-'));
const chrome = process.env.CHROME_BIN ?? (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : 'google-chrome');
const environment = {...process.env, CARGO_PROFILE_DEV_DEBUG: '0', CARGO_INCREMENTAL: '0'};
const bootstrap = `
import sys, json, subprocess, os
from pathlib import Path
root, work = Path(sys.argv[1]), Path(sys.argv[2])
cli = Path(os.environ.get('RAKE_CLI', str(Path(os.environ.get('CARGO_TARGET_DIR', str(root / 'target'))) / 'debug/rake')))
for label in ['old', 'new']:
    site = work / (label + '-source')
    site.mkdir()
    (site / 'platform.js').write_text((root / 'crates/rake/src/platform/browser.js').read_text())
    (site / 'runtime.js').write_text((root / 'crates/rake/src/web/browser.js').read_text())
    (site / 'count.worker.js').write_text('self.onmessage=e=>self.postMessage(JSON.stringify({length:JSON.parse(e.data).text.length,build:' + json.dumps(label) + '}));')
    (site / 'never.worker.js').write_text('self.onmessage=()=>{};')
    (site / 'index.html').write_text('<html><head></head><body><button id="pick">Pick file</button><script type="module">import * as platform from "./platform.js"; import * as runtime from "./runtime.js"; import * as fixture from "./pkg/rake_web_host_fixture.js"; await fixture.default();fixture.start();window.fixture=fixture;window.platform=platform;window.runtime=runtime;window.workersStarted=0;window.workersTerminated=0;window.Worker=class extends window.Worker {constructor(...args){super(...args);window.workersStarted++;}terminate(){window.workersTerminated++;super.terminate();}};window.build=' + json.dumps(label) + ';document.querySelector("#pick").onclick=async()=>{try{const f=await platform.pick_file();window.picked=f?{name:f.name,text:new TextDecoder().decode(await platform.read_file(f,100))}:null;}catch(e){window.pickError=e.message;}};window.ready=true;</script></body></html>')
    subprocess.run([str(cli), 'bundle', '--target', 'web', '--manifest-path', str(root / 'tests/web-platform/Cargo.toml'), '--web-dir', str(site), '--output', str(work / label), '--json'], cwd=root, check=True)

`;
const generated = spawnSync('python3', ['-c', bootstrap, root, temporary], {encoding: 'utf8', env: environment});
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
      try { path = join(temporary, build, 'site', relative || 'index.html'); data = await readFile(path); break; } catch (_) { /* Immutable old build fallback. */ }
    }
    if (!data && !relative.includes('.')) { path = join(temporary, current, 'site', 'index.html'); data = await readFile(path); }
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
let cdp;
const events = [];
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
  return cdp.call(method, params, sessionId);
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
  cdp = createBrowserCdp(socket, processChrome, {
    stderr: () => stderr,
    onEvent: event => { events.push(event); if (events.length > 1000) events.shift(); },
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
  cdp?.close();
  socket?.close();
  processChrome.kill('SIGTERM');
  await Promise.race([new Promise(resolve => processChrome.once('exit', resolve)), delay(5000)]);
  if (processChrome.exitCode === null) processChrome.kill('SIGKILL');
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
  await rm(temporary, {recursive: true, force: true});
}
