#!/usr/bin/env node
// Experimental split view for an existing `rake dev --target web --cdp` session.
// Requires Node 22+, a live Web build, and network access for uncached Chromium assets.
import {randomBytes} from 'node:crypto';
import {createServer} from 'node:http';
import {connect} from 'node:net';
import {mkdir, readFile, writeFile, realpath} from 'node:fs/promises';
import {join} from 'node:path';
import {pathToFileURL} from 'node:url';
import {parseArgs} from 'node:util';

// Matches the Chrome frontend used for this experiment; never fetch credentials remotely.
export const FRONTEND_REVISION = 'fb096c0011f9d50f0a264a9376420cdea9c421e3';
const FRONTEND = `https://chrome-devtools-frontend.appspot.com/serve_file/@${FRONTEND_REVISION}/`;
const mime = {'.html':'text/html', '.js':'text/javascript', '.css':'text/css', '.json':'application/json',
  '.svg':'image/svg+xml', '.png':'image/png', '.woff2':'font/woff2', '.wasm':'application/wasm'};

function loopback(value, protocol) {
  const url = new URL(value);
  if (url.protocol !== protocol || !['localhost', '127.0.0.1'].includes(url.hostname) || url.username || url.password) {
    throw new Error(`Expected a ${protocol} loopback URL`);
  }
  return url;
}

export async function startEmbeddedDevtools({session, appUrl, port = 0}) {
  const app = loopback(appUrl, 'http:');
  if (!new URLSearchParams(app.hash.slice(1)).has('__rake_control')) throw new Error('Use the private app launch URL, including its control fragment');
  if (!Number.isInteger(port) || port < 0 || port > 65535) throw new Error('Invalid port');
  const state = JSON.parse(await readFile(join(session, 'status.json'), 'utf8'));
  if (state.target !== 'web' || !state.published || !state.directory) throw new Error('A published Rake Web build is required');
  const cdp = loopback(JSON.parse(await readFile(join(session, 'cdp.json'), 'utf8')).url, 'http:');
  const cache = join(session, 'embedded-devtools', FRONTEND_REVISION);
  await mkdir(cache, {recursive:true, mode:0o700});
  const token = randomBytes(24).toString('hex');
  const prefix = `/${token}/`;
  const sockets = new Set(), pendingAssets = new Map();
  let origin;

  async function asset(path) {
    if (!pendingAssets.has(path)) {
      const download = (async () => {
        const file = join(cache, path);
        try { return await readFile(file); } catch (error) { if (error.code !== 'ENOENT') throw error; }
        const response = await fetch(FRONTEND + path, {signal:AbortSignal.timeout(20000), redirect:'error'});
        if (!response.ok) throw new Error(`Chromium asset ${path}: HTTP ${response.status}`);
        const body = Buffer.from(await response.arrayBuffer());
        if (body.length > 16 * 1024 * 1024) throw new Error('Chromium asset exceeds 16 MiB');
        await mkdir(join(file, '..'), {recursive:true, mode:0o700});
        await writeFile(file, body, {mode:0o600});
        return body;
      })();
      pendingAssets.set(path, download);
      download.catch(() => pendingAssets.delete(path));
    }
    return pendingAssets.get(path);
  }

  const server = createServer(async (request, response) => {
    const path = request.url?.split('?')[0];
    if (request.headers.host !== new URL(origin).host || request.method !== 'GET' || !path?.startsWith(prefix + 'frontend/')) {
      response.writeHead(403).end('Private local DevTools assets only');
      return;
    }
    const relative = path.slice((prefix + 'frontend/').length);
    const extension = relative.slice(relative.lastIndexOf('.'));
    if (!/^[a-zA-Z0-9_./-]+$/.test(relative) || relative.split('/').some(part => part === '..' || part === '.') || !mime[extension]) {
      response.writeHead(404).end('Unknown frontend asset');
      return;
    }
    try {
      const body = await asset(relative);
      response.writeHead(200, {'Content-Type':mime[extension], 'Content-Length':body.length,
        'Cache-Control':'no-store', 'Referrer-Policy':'no-referrer', 'X-Content-Type-Options':'nosniff'}).end(body);
    } catch (error) {
      console.error(error.message);
      response.writeHead(502, {'Content-Type':'text/plain'}).end('Unable to load the pinned Chromium frontend asset');
    }
  });
  server.on('upgrade', async (request, socket, head) => {
    socket.on('error', () => socket.destroy());
    const runtime = request.url?.slice((prefix + 'cdp/').length);
    if (request.headers.host !== new URL(origin).host ||
        (request.headers.origin && request.headers.origin !== origin) ||
        !request.url?.startsWith(prefix + 'cdp/') || !/^[a-f0-9-]{36}$/.test(runtime) ||
        request.headers.upgrade?.toLowerCase() !== 'websocket' ||
        request.headers['sec-websocket-version'] !== '13' || sockets.size >= 8) {
      socket.end('HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n');
      return;
    }
    sockets.add(socket);
    socket.on('close', () => sockets.delete(socket));
    try {
      const discovery = await fetch(new URL('/json/list', cdp), {signal:AbortSignal.timeout(5000)});
      if (!discovery.ok) throw new Error('CDP discovery unavailable');
      const target = (await discovery.json()).find(target => target.id === runtime);
      if (!target) throw new Error('Embedded app runtime is not advertised');
      const upstream = loopback(target.webSocketDebuggerUrl, 'ws:');
      if (upstream.host !== cdp.host) throw new Error('CDP discovery changed servers');
      if (socket.destroyed) return;
      const remote = connect({host:upstream.hostname, port:Number(upstream.port)});
      remote.setTimeout(5000, () => remote.destroy());
      remote.once('data', () => remote.setTimeout(0));
      remote.on('error', () => socket.destroy());
      remote.on('close', () => socket.destroy());
      socket.on('close', () => remote.destroy());
      remote.once('connect', () => {
        remote.write(`GET ${upstream.pathname} HTTP/1.1\r\nHost: ${upstream.host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nOrigin: devtools://devtools\r\nSec-WebSocket-Key: ${request.headers['sec-websocket-key']}\r\nSec-WebSocket-Version: 13\r\n\r\n`);
        if (head.length) remote.write(head);
        socket.pipe(remote).pipe(socket);
      });
    } catch (error) {
      console.error(error.message);
      socket.end('HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n');
    }
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(port, '127.0.0.1', resolve);
  });
  origin = `http://127.0.0.1:${server.address().port}`;
  try {
    const template = await readFile(new URL('./embedded-devtools.html', import.meta.url), 'utf8');
    const config = JSON.stringify({appUrl:app.href, frontend:`${origin}${prefix}frontend/devtools_app.html`,
      socket:`${new URL(origin).host}${prefix}cdp/`}).replaceAll('<', '\\u003c');
    const filename = `embedded-devtools-${token}.html`;
    await writeFile(join(state.directory, filename), template.replace('/* EMBEDDED_CONFIG */', config), {mode:0o600});
    const url = new URL(`/__rake/build/${state.published}/${filename}`, app.origin).href;
    return {url, origin, prefix, close:() => {
      for (const socket of sockets) socket.destroy();
      server.closeAllConnections();
      return new Promise(resolve => server.close(resolve));
    }};
  } catch (error) {
    server.closeAllConnections();
    server.close();
    throw error;
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(await realpath(process.argv[1])).href) {
  const {values} = parseArgs({options:{session:{type:'string'}, 'app-url':{type:'string'}, port:{type:'string', default:'0'}, json:{type:'boolean'}}});
  if (!values.session || !values['app-url']) {
    console.error('Usage: node crates/rake-cli/scripts/embedded-devtools.mjs --session /tmp/rake-dev-EXAMPLE --app-url PRIVATE_LAUNCH_URL [--port 0]');
    process.exitCode = 1;
  } else {
    try {
      const demo = await startEmbeddedDevtools({session:values.session, appUrl:values['app-url'], port:Number(values.port)});
      console.log(values.json ? JSON.stringify({event:'embedded_devtools_listening', url:demo.url}) : `Embedded Chrome DevTools: ${demo.url}`);
      for (const signal of ['SIGINT', 'SIGTERM']) process.once(signal, () => demo.close().then(() => process.exit()));
    } catch (error) {
      console.error(error.message);
      process.exitCode = 1;
    }
  }
}
