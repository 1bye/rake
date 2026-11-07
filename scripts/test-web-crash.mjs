// Covers real Rust App startup, panic persistence and next-load delivery in Chrome.
// Requires Node 22+, Chrome, pinned Rust/wasm target and matching WASM_BINDGEN.
import assert from 'node:assert/strict';
import {spawn, spawnSync} from 'node:child_process';
import {createServer} from 'node:http';
import {mkdtemp, readFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {setTimeout as delay} from 'node:timers/promises';
import {createBrowserCdp} from './browser-test-cdp.mjs';

const root = fileURLToPath(new URL('../', import.meta.url));
const temporary = await mkdtemp(join(tmpdir(), 'rake-web-crash-'));
const chrome = process.env.CHROME_BIN ?? (process.platform === 'darwin' ? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome' : 'google-chrome');
let processChrome, socket, cdp, server;
let stderr = '';
const events = [];
async function eventually(check, timeout = 30_000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    try { const value = await check(); if (value) return value; } catch (error) {
      if (!String(error).includes('Cannot find context')) throw error;
    }
    if (processChrome?.exitCode != null) throw new Error(stderr);
    await delay(30);
  }
  throw new Error('Browser crash check timed out: ' + stderr);
}
const call = (method, params = {}, sessionId) => cdp.call(method, params, sessionId);
async function evaluate(session, expression) {
  const reply = await call('Runtime.evaluate', {expression, returnByValue: true, awaitPromise: true}, session);
  if (reply.exceptionDetails) throw new Error(JSON.stringify(reply.exceptionDetails));
  return reply.result.value;
}
async function page(mode = 'ok', include = false, bytes = 65536) {
  const {targetId} = await call('Target.createTarget', {url: 'about:blank'});
  const {sessionId} = await call('Target.attachToTarget', {targetId, flatten: true});
  await call('Page.enable', {}, sessionId);
  await call('Runtime.enable', {}, sessionId);
  await call('Page.navigate', {url: origin + '/?mode=' + mode + '&include=' + include + '&bytes=' + bytes}, sessionId);
  await eventually(() => evaluate(sessionId, 'window.ready === true || window.startError'));
  assert.equal(await evaluate(sessionId, 'window.startError ?? null'), null);
  return sessionId;
}
async function close(session) {
  const {targetInfo} = await call('Target.getTargetInfo', {}, session);
  await call('Target.closeTarget', {targetId: targetInfo.targetId});
}
const prefix = 'rake:crashes:test.web-crash:';
const records = 'Object.keys(localStorage).filter(k => k.startsWith(' + JSON.stringify(prefix) + ')).sort().map(k => { const value=localStorage[k]; const modern=/^[PC][0-3]:/.test(value); const raw=value.slice(modern?3:1); return {key:k,state:value[0],attempts:modern?Number(value[1]):(value[0]==="C"?1:0),raw,record:JSON.parse(raw)}; })';
const receipts = 'JSON.parse(localStorage.getItem("receipts") || "[]")';
async function panic(session, large = false) {
  assert.equal(await evaluate(session, '(() => { try { fixture.panic_now(' + large + '); return false; } catch (_) { return true; } })()'), true, 'Rust panic traps');
}
let origin;
try {
  const built = spawnSync('cargo', ['build', '-p', 'rake-app', '--example', 'web-crash', '--target', 'wasm32-unknown-unknown', '--locked'], {cwd: root, stdio: 'inherit'});
  assert.equal(built.status, 0, 'build actual App fixture');
  const bindgen = process.env.WASM_BINDGEN ?? 'wasm-bindgen';
  const lock = await readFile(join(root, 'Cargo.lock'), 'utf8');
  const expected = lock.split('name = "wasm-bindgen"')[1].split('"')[1];
  assert.equal(spawnSync(bindgen, ['--version'], {encoding: 'utf8'}).stdout?.trim(), 'wasm-bindgen ' + expected, 'matching wasm-bindgen required');
  const target = resolve(root, process.env.CARGO_TARGET_DIR ?? 'target');
  const generated = spawnSync(bindgen, ['--target', 'web', '--out-name', 'fixture', '--out-dir', temporary, join(target, 'wasm32-unknown-unknown/debug/examples/web_crash.wasm')], {stdio: 'inherit'});
  assert.equal(generated.status, 0, 'generate bindings');
  const bootstrap = '<html><body><script type="module">import * as fixture from "/fixture.js"; window.fixture=fixture; await fixture.default(); const q=new URLSearchParams(location.search);try { fixture.start(q.get("mode"),q.get("include")==="true",Number(q.get("bytes"))); } catch(e) {window.startError=String(e);}window.ready=true;</script></body></html>';
  server = createServer(async (request, response) => {
    const path = new URL(request.url, 'http://localhost').pathname;
    try {
      if (path === '/') { response.writeHead(200, {'Content-Type':'text/html'}).end(bootstrap); return; }
      if (path === '/adapter.js') { response.writeHead(200, {'Content-Type':'text/javascript'}).end(await readFile(join(root, 'crates/rake/src/crash/browser.js'))); return; }
      assert(!path.split('/').includes('..'));
      const data = await readFile(join(temporary, path));
      response.writeHead(200, {'Content-Type':path.endsWith('.wasm')?'application/wasm':'text/javascript'}).end(data);
    } catch (_) { response.writeHead(404).end(); }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  origin = 'http://127.0.0.1:' + server.address().port;
  const profile = join(temporary, 'profile');
  processChrome = spawn(chrome, ['--headless=new', '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--remote-debugging-port=0', '--user-data-dir=' + profile, 'about:blank'], {stdio:['ignore','ignore','pipe']});
  processChrome.stderr.on('data', data => { stderr = (stderr + data).slice(-8000); });
  const port = await eventually(async () => { try { return (await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; } catch (_) { return null; } });
  const browser = await (await fetch('http://127.0.0.1:' + port + '/json/version')).json();
  socket = new WebSocket(browser.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.addEventListener('open', resolve, {once:true}); socket.addEventListener('error', reject, {once:true}); });
  cdp = createBrowserCdp(socket, processChrome, {stderr:() => stderr, onEvent:event => { events.push(event); }});

  let session = await page('automatic');
  await panic(session);
  let pending = await eventually(async () => { const r = await evaluate(session, records); return r.length === 1 && r; });
  assert.equal(pending[0].record.message, 'Rust panic (payload omitted for privacy)');
  assert.equal(pending[0].record.app_version, 'consumer-web-1.2.3');
  assert(!pending[0].raw.includes('SECRET_USER_CONTENT'));
  assert(events.some(e => e.method === 'Runtime.consoleAPICalled' && e.params.type === 'error'), 'console panic hook retained');
  const id = pending[0].record.id;
  await close(session);

  for (let load=0;load<4;load++) {
    session = await page('fail');
    await eventually(() => evaluate(session, 'window.sinkAttempts === 1'));
    pending = await evaluate(session, records);
    assert.equal(pending[0].record.id, id, 'failing Rust sink retains original');
    assert.equal(pending[0].state, 'P');
    assert.equal(pending[0].attempts, 0, 'ordinary Err never consumes the interrupted-attempt budget');
    assert.equal((await evaluate(session, receipts)).length, 0);
    await close(session);
  }
  const tabs = await Promise.all([page(), page()]);
  await eventually(() => evaluate(tabs[0], receipts + '.length === 1'));
  assert.equal((await evaluate(tabs[0], receipts))[0].id, id);
  assert.equal((await evaluate(tabs[0], records)).length, 0, 'two startup tabs deliver once');
  await Promise.all(tabs.map(close));
  session = await page();
  assert.equal((await evaluate(session, receipts)).length, 1, 'later load does not redeliver');
  await evaluate(session, 'localStorage.clear()');
  await close(session);

  // Real wasm serialization: opted-in huge Unicode/escaping fits the minimum budget.
  session = await page('ok', true, 1024);
  await panic(session, true);
  pending = await eventually(async () => { const r = await evaluate(session, records); return r.length === 1 && r; });
  assert(Buffer.byteLength(pending[0].raw) <= 1024);
  assert(pending[0].record.truncated);
  assert(Buffer.byteLength(pending[0].record.message) <= 1024);
  const boundedId = pending[0].record.id;
  // Test interrupted claim, then a new page retries it.
  await evaluate(session, 'localStorage[' + JSON.stringify(pending[0].key) + '] = "C1:" + ' + JSON.stringify(pending[0].raw));
  await close(session);
  session = await page();
  await eventually(() => evaluate(session, receipts + '.length === 1'));
  assert.equal((await evaluate(session, receipts))[0].id, boundedId);
  await evaluate(session, 'localStorage.clear()');
  await close(session);

  // Two real records survive repeated sink panics even at the two-record limit.
  for (let i=0;i<2;i++) { session = await page('automatic'); await panic(session); await close(session); }
  session = await page('disabled');
  const originalIds = (await evaluate(session, records)).map(record => record.record.id);
  assert.equal(originalIds.length, 2);
  await close(session);
  for (let load=1;load<=4;load++) {
    session = await page('unwind');
    await eventually(() => evaluate(session, 'window.sinkAttempts === 1'));
    pending = await evaluate(session, records);
    assert.deepEqual(await evaluate(session, 'window.sinkIds'), [originalIds[load < 4 ? 0 : 1]], 'no sink-panic record is delivered in its own pass');
    const warnings = await evaluate(session, 'JSON.parse(fixture.warnings())');
    const abandoned = warnings.filter(warning => warning.message === 'crash record discarded after three interrupted deliveries');
    if (load < 4) {
      assert.deepEqual(pending.map(record => record.record.id), originalIds, 'sink panic must not evict either real record');
      assert.equal(pending[0].state, 'C');
      assert.equal(pending[0].attempts, load);
      assert.equal(abandoned.length, 0);
    } else {
      assert(!pending.some(record => record.record.id === originalIds[0]), 'three interrupted attempts exhaust the record');
      assert(pending.some(record => record.record.id === originalIds[1]), 'the other real record survives');
      assert.equal(abandoned.length, 1);
      assert.deepEqual(abandoned[0], {message:'crash record discarded after three interrupted deliveries', record_id:originalIds[0]}, 'warning contains only the record ID and fixed text');
    }
    await close(session);
  }
  session = await page();
  await eventually(() => evaluate(session, receipts + '.length === 2'));
  const remaining = await evaluate(session, receipts);
  assert(remaining.some(record => record.id === originalIds[1]), 'healthy sink receives the remaining real record');
  assert(!remaining.some(record => record.id === originalIds[0]), 'exhausted record is never sent again');
  assert.equal((await evaluate(session, records)).length, 0);
  await close(session);
  session = await page();
  assert.equal((await evaluate(session, receipts)).length, 2);
  assert.equal((await evaluate(session, 'JSON.parse(fixture.warnings())')).filter(warning => warning.record_id === originalIds[0]).length, 0, 'drop warning is emitted once');
  await evaluate(session, 'localStorage.clear()');
  await close(session);

  // Multiple actual panic pages retain only two records without a sink.
  for (let i=0;i<4;i++) { session = await page('fail'); await panic(session); await close(session); }
  session = await page('disabled');
  await eventually(() => evaluate(session, records + '.length === 2'));
  await evaluate(session, 'localStorage.clear()');
  await panic(session);
  assert.equal((await evaluate(session, records)).length, 0, 'opt-out leaves console hook only');
  await close(session);

  // Production JS boundary: real storage/Web Locks, fault injection only for failures.
  session = await page('disabled');
  await evaluate(session, 'import("/adapter.js").then(adapter => { window.adapter = adapter; })');
  const hostChecks = await evaluate(session, '(async () => {' +
    'const p="rake:crashes:adapter-test:"; const raw=JSON.stringify({schema_version:2,future:"preserve"});' +
    'adapter.crash_write(p,"001",raw,1024,2);await navigator.locks.request(p+"spool",()=>{});' +
    'const trapped=await adapter.crash_deliver(p,1024,2,()=>{throw Error("sink")});' +
    'if(trapped!==-2||localStorage[p+"001"]!=="C1:"+raw)throw Error("throwing sink lost record");' +
    'await adapter.crash_deliver(p,1024,2,()=>0);if(localStorage[p+"001"]!=="P1:"+raw)throw Error("future schema lost");' +
    'const original=Storage.prototype.setItem;Storage.prototype.setItem=()=>{throw Error("quota")};try{adapter.crash_write(p,"002",raw,1024,2)}finally{Storage.prototype.setItem=original}' +
    'const huge="🦀".repeat(300);adapter.crash_write(p,"003",huge,1024,2);if(localStorage[p+"003"])throw Error("encoded bytes");' +
    'await navigator.locks.request(p+"spool",async()=>{if(await adapter.crash_deliver(p,1024,2,()=>{throw Error("busy sink")}))throw Error("busy")});' +
    'Object.defineProperty(navigator,"locks",{value:undefined,configurable:true});' +
    'let unsupported=false;try{await adapter.crash_deliver(p,1024,2,()=>{throw Error("no-lock sink")})}catch{unsupported=true}' +
    'if(!unsupported||!localStorage[p+"001"])throw Error("no-lock record lost");' +
    'return true;})()');
  assert(hostChecks);
  await close(session);
  console.log(JSON.stringify({browser:browser.Browser, passed:['real App panic and next-load Rust sink', 'default privacy and consumer version', 'console hook', 'failing Rust sink retry', 'two simultaneous tabs deliver once', 'later launch stays empty', 'minimum byte budget with Unicode/escaping', 'interrupted claim recovery', 'count bound across panic pages', 'builder opt-out', 'three interrupted attempts discard once without evicting real records', 'sink traps retain claims', 'future schema preservation', 'quota exception is nonfatal', 'UTF-8 byte rejection', 'busy lock skips sink', 'no-lock delivery defers']}));
} finally {
  cdp?.close(); socket?.close();
  if (processChrome) {
    processChrome.kill('SIGTERM');
    await Promise.race([new Promise(resolve => processChrome.once('exit', resolve)), delay(5000)]);
    if (processChrome.exitCode === null) processChrome.kill('SIGKILL');
  }
  if (server) { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
  await rm(temporary, {recursive:true, force:true});
}
