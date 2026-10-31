// Real service-worker handlers with an in-memory cache/network host. No lifecycle/GPU claim.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';

const source = await readFile(new URL('./web-service-worker.js', import.meta.url), 'utf8');
const origin = 'https://example.test';
const cached = new Map();
let network = new Map();
let fail = false;
const urlOf = request => typeof request === 'string' ? new URL(request, origin).href : request.url;
const fetch = async request => {
  if (fail) throw new Error('offline');
  const value = network.get(urlOf(request));
  if (value === undefined) throw new Error('missing deployment asset');
  return new Response(value);
};
const caches = {
  async open(key) {
    if (!cached.has(key)) cached.set(key, new Map());
    const contents = cached.get(key);
    return {
      async addAll(requests) {
        const complete = await Promise.all(requests.map(async request => [urlOf(request), await (await fetch(request)).text()]));
        for (const [url, text] of complete) contents.set(url, text);
      },
      async match(request) { const text = contents.get(urlOf(request)); return text === undefined ? undefined : new Response(text); },
    };
  },
  async keys() { return [...cached.keys()]; },
  async delete(key) { return cached.delete(key); },
  async match(request) {
    for (const contents of cached.values()) {
      if (contents.has(urlOf(request))) return new Response(contents.get(urlOf(request)));
    }
  },
};
function worker(build) {
  const handlers = new Map();
  const base = `/tools/rake-builds/${build}/`;
  const index = base + 'index.html';
  const assets = [index, base + 'app.js'];
  const config = `const CACHE='rake:test:${build}', PREFIX='rake:test:', ROOT='/tools/', INDEX=${JSON.stringify(index)}, ASSETS=${JSON.stringify(assets)};\n`;
  vm.runInNewContext(config + source, {
    URL, Response, Request: class { constructor(path) { this.url = new URL(path, origin).href; } },
    fetch, caches, self: {location: {origin}, addEventListener: (type, listener) => handlers.set(type, listener), skipWaiting() { assert.fail('must not skip waiting'); }},
  });
  return {
    async lifecycle(type) { let work; handlers.get(type)({waitUntil: value => { work = value; }}); return work; },
    request(path, mode = 'cors', method = 'GET') {
      let work;
      handlers.get('fetch')({request: {url: new URL(path, origin).href, mode, method}, respondWith: value => { work = value; }});
      return work;
    },
    index,
    base,
  };
}
const old = worker('old');
network.set(origin + old.index, 'old shell');
network.set(origin + old.base + 'app.js', 'old module');
await old.lifecycle('install');
network.set(origin + '/tools/document/1', 'new shell');
assert.equal(await (await old.request('/tools/document/1', 'navigate')).text(), 'new shell');
fail = true;
assert.equal(await (await old.request('/tools/document/1', 'navigate')).text(), 'old shell');
assert.equal(await (await old.request(old.base + 'app.js')).text(), 'old module');
assert.equal(old.request('/tools/rake-version.json'), undefined);
assert.equal(old.request('/tools/api/private'), undefined);
assert.equal(old.request('/tools/api/private', 'cors', 'POST'), undefined);
assert.equal(old.request('https://other.test/tools/a', 'navigate'), undefined);
const next = worker('next');
await assert.rejects(next.lifecycle('install'), /offline/);
assert.equal(cached.has('rake:test:next'), false, 'partial cache must be discarded');
assert.equal(cached.has('rake:test:old'), true);
fail = false;
network.set(origin + next.index, 'next shell');
network.set(origin + next.base + 'app.js', 'next module');
await next.lifecycle('install');
assert.equal(await (await old.request(old.base + 'app.js')).text(), 'old module', 'installing an update must preserve a live tab');
cached.set('another-app:cache', new Map());
await next.lifecycle('activate');
assert.equal(cached.has('rake:test:old'), false);
assert.equal(cached.has('another-app:cache'), true);
fail = true;
assert.equal(await (await next.request('/tools/deep/link', 'navigate')).text(), 'next shell');
console.log('Offline handlers: complete installs, immutable builds, navigation fallback, API exclusion and scoped cache cleanup passed');
