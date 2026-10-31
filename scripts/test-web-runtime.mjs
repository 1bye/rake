// Real browser worker/deployment adapter with deterministic host doubles.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const source = await readFile(new URL('../crates/rake/src/web/browser.js', import.meta.url), 'utf8');
const adapter = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
globalThis.document = {baseURI: 'https://example.test/tools/rake-builds/old/', querySelector: () => ({content: '{"build":"old"}'})};
globalThis.location = {origin: 'https://example.test'};
assert.equal(adapter.deployment_metadata(), '{"build":"old"}');
let terminated = 0;
let mode = 'success';
globalThis.Worker = class {
  constructor(url, options) { assert.equal(url.origin, location.origin); assert.equal(options.type, 'module'); }
  postMessage(raw) {
    if (mode === 'wait') return;
    queueMicrotask(() => {
      if (mode === 'error') this.onerror({message: 'worker crashed', preventDefault() {}});
      else this.onmessage({data: mode === 'malformed' ? 'broken' : JSON.stringify({length: JSON.parse(raw).text.length})});
    });
  }
  terminate() { terminated++; }
};
const job = adapter.start_worker('count.worker.js', '{"text":"hello"}', 1000);
assert.deepEqual(JSON.parse(await adapter.worker_promise(job)), {length: 5});
adapter.cancel_worker(job);
assert.equal(terminated, 1, 'success terminates exactly once');
mode = 'malformed';
await assert.rejects(adapter.worker_promise(adapter.start_worker('count.worker.js', '{}', 1000)), /invalid JSON/);
mode = 'error';
await assert.rejects(adapter.worker_promise(adapter.start_worker('count.worker.js', '{}', 1000)), /crashed/);
mode = 'wait';
const cancelled = adapter.start_worker('count.worker.js', '{}', 1000);
const cancelledResult = adapter.worker_promise(cancelled);
adapter.cancel_worker(cancelled);
await assert.rejects(cancelledResult, {name: 'AbortError'});
await assert.rejects(adapter.worker_promise(adapter.start_worker('count.worker.js', '{}', 1)), {name: 'TimeoutError'});
assert.equal(terminated, 5);
assert.throws(() => adapter.start_worker('https://other.test/worker.js', '{}', 1000), /same-origin/);
assert.throws(() => adapter.start_worker('worker.js', 'x'.repeat(1_048_577), 1000), /1 MiB/);
assert.throws(() => adapter.start_worker('worker.js', '{}', 0), /timeout/);

globalThis.fetch = async (url, options) => {
  assert.equal(url.href, 'https://example.test/tools/rake-version.json');
  assert.equal(options.cache, 'no-store');
  assert.equal(options.redirect, 'error');
  return new Response('{"version":"2"}');
};
assert.equal(await adapter.check_deployment('/tools/'), '{"version":"2"}');
globalThis.fetch = async () => new Response('x'.repeat(16_385));
await assert.rejects(adapter.check_deployment('/tools/'), /16 KiB/);
globalThis.fetch = async () => new Response('error', {status: 503});
await assert.rejects(adapter.check_deployment('/tools/'), /503/);
console.log('Browser runtime adapter: worker ownership, cancellation, timeout, bounded replies and uncached deployment checks passed');
