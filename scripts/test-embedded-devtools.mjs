// Covers: private asset serving, exact runtime selection, WebSocket forwarding and origin rejection.
// Real: HTTP/TCP, filesystem and proxy. Doubles: cached frontend HTML and a CDP discovery/upgrade server.
// Requires: Node 22+ and loopback sockets. The actual Chromium UI is verified separately in Chrome.
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {spawn} from 'node:child_process';
import {createServer, get} from 'node:http';
import {mkdtemp, mkdir, writeFile, readFile, rm, symlink} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {test} from 'node:test';
import {FRONTEND_REVISION, startEmbeddedDevtools} from '../crates/rake-cli/scripts/embedded-devtools.mjs';

test('embedded DevTools keeps the proxy private and forwards only the selected local runtime', {timeout:10000}, async () => {
  const session = await mkdtemp(join(tmpdir(), 'rake-embedded-test-'));
  const runtime = '11111111-2222-4333-8444-555555555555';
  const build = join(session, 'builds', '1');
  const cache = join(session, 'embedded-devtools', FRONTEND_REVISION);
  const forwarded = [], connections = new Set();
  let authority;
  const upstream = createServer((request, response) => {
    assert.equal(request.url, '/json/list');
    response.setHeader('Content-Type', 'application/json');
    response.end(JSON.stringify([{id:runtime, webSocketDebuggerUrl:`ws://${authority}/private/${runtime}`} ]));
  });
  upstream.on('upgrade', (request, socket) => {
    connections.add(socket);
    socket.on('error', () => socket.destroy());
    socket.on('close', () => connections.delete(socket));
    forwarded.push({url:request.url, host:request.headers.host, origin:request.headers.origin});
    const accept = createHash('sha1').update(request.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    // An actual server WebSocket text frame, forwarded unchanged by the proxy.
    socket.write(Buffer.from([0x81, 2, 0x6f, 0x6b]));
  });
  await new Promise(resolve => upstream.listen(0, '127.0.0.1', resolve));
  authority = `127.0.0.1:${upstream.address().port}`;
  let demo;
  try {
    await mkdir(build, {recursive:true}); await mkdir(cache, {recursive:true});
    await writeFile(join(cache, 'devtools_app.html'), '<!doctype html><title>Cached test frontend</title>');
    await writeFile(join(session, 'status.json'), JSON.stringify({target:'web', published:1, directory:build}));
    await writeFile(join(session, 'cdp.json'), JSON.stringify({url:`http://${authority}`}));
    demo = await startEmbeddedDevtools({session, appUrl:'http://localhost:8093/#__rake_control=test-only'});

    const wrapper = await readFile(join(build, new URL(demo.url).pathname.split('/').at(-1)), 'utf8');
    assert.ok(wrapper.includes('http://localhost:8093/#__rake_control=test-only'));
    assert.ok(!wrapper.includes('/* EMBEDDED_CONFIG */'));
    const frontend = `${demo.origin}${demo.prefix}frontend/devtools_app.html`;
    assert.equal(await (await fetch(frontend)).text(), '<!doctype html><title>Cached test frontend</title>');
    assert.equal((await fetch(demo.origin + '/frontend/devtools_app.html')).status, 403);
    const rejectedHost = await new Promise((resolve, reject) => {
      get(frontend, {headers:{Host:'evil.test'}}, response => {
        response.resume(); resolve(response.statusCode);
      }).on('error', reject);
    });
    assert.equal(rejectedHost, 403);
    assert.equal((await fetch(`${demo.origin}${demo.prefix}frontend/%2e%2e/status.json`)).status, 403);

    async function upgrade(origin, id = runtime) {
      return new Promise((resolve, reject) => {
        const request = get(`${demo.origin}${demo.prefix}cdp/${id}`, {headers:{
          Origin:origin, Upgrade:'websocket', Connection:'Upgrade',
          'Sec-WebSocket-Key':'dGhlIHNhbXBsZSBub25jZQ==', 'Sec-WebSocket-Version':'13',
        }});
        request.on('error', reject);
        request.on('response', response => { response.resume(); resolve({status:response.statusCode}); });
        request.on('upgrade', (response, socket, head) => {
          const finish = body => { socket.destroy(); resolve({status:response.statusCode, body}); };
          if (head.length) finish(head);
          else socket.once('data', finish);
        });
      });
    }
    assert.equal((await upgrade('https://evil.test')).status, 403);
    assert.equal((await upgrade(demo.origin, 'aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee')).status, 502);
    const accepted = await upgrade(demo.origin);
    assert.equal(accepted.status, 101);
    assert.deepEqual([...accepted.body], [0x81, 2, 0x6f, 0x6b]);
    assert.deepEqual(forwarded, [{url:`/private/${runtime}`, host:authority, origin:'devtools://devtools'}]);
    await assert.rejects(startEmbeddedDevtools({session, appUrl:'https://example.com/#__rake_control=x'}), /loopback/);

    // macOS temporary helpers can be launched through /var while Node resolves /private/var.
    const alias = join(session, 'helper-alias');
    await symlink(new URL('../crates/rake-cli/scripts/', import.meta.url), alias, 'dir');
    const child = spawn(process.execPath, [join(alias, 'embedded-devtools.mjs'), '--session', session,
      '--app-url', 'http://localhost:8093/#__rake_control=test-only', '--json'], {stdio:['ignore', 'pipe', 'inherit']});
    try {
      const ready = await new Promise((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error('aliased helper did not become ready')), 3000);
        child.once('error', error => { clearTimeout(timer); reject(error); });
        child.once('exit', code => { clearTimeout(timer); reject(new Error(`aliased helper exited before readiness: ${code}`)); });
        child.stdout.once('data', bytes => { clearTimeout(timer); resolve(JSON.parse(bytes)); });
      });
      assert.equal(ready.event, 'embedded_devtools_listening');
    } finally {
      child.kill();
      if (child.exitCode === null && child.signalCode === null) await new Promise(resolve => child.once('exit', resolve));
    }
  } finally {
    if (demo) await demo.close();
    for (const socket of connections) socket.destroy();
    upstream.closeAllConnections();
    await new Promise(resolve => upstream.close(resolve));
    await rm(session, {recursive:true, force:true});
  }
});
