// Real browser adapter functions; in-memory storage/lock host, no browser/GPU claim.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../examples/notes/src/persistence/browser.rs', import.meta.url), 'utf8');
const inline = source.split('inline_js = r#"')[1].split('"#)]')[0];
const { fieldnotes_read: read, fieldnotes_write: write } = await import(`data:text/javascript;base64,${Buffer.from(inline).toString('base64')}`);
const values = new Map();
let failKey;
globalThis.localStorage = {
  getItem: key => values.get(key) ?? null,
  setItem(key, value) {
    if (key === failKey) throw new Error('Quota exceeded');
    values.set(key, value);
  },
};
let queue = Promise.resolve();
Object.defineProperty(globalThis, 'navigator', { value: { locks: {
  request(key, options, work) {
    assert.equal(key, 'test');
    assert.ok(['shared', 'exclusive'].includes(options.mode));
    const pending = queue.then(work);
    queue = pending.catch(() => {});
    return pending;
  },
} }, configurable: true });
const raw = (revision, writer, title) => JSON.stringify({format: 1, revision, writer, library: {notes: [{id: 1, title}]}});
const first = raw(1, 'a', 'First');
await write('test', first, 0, false);
assert.deepEqual(JSON.parse(await read('test')), [first, first, null]);
const results = await Promise.allSettled([
  write('test', raw(2, 'b', 'Second'), 1, false),
  write('test', raw(2, 'c', 'Concurrent'), 1, false),
]);
assert.equal(results.filter(result => result.status === 'fulfilled').length, 1);
assert.match(results.find(result => result.status === 'rejected').reason.message, /CONFLICT/);
assert.equal(JSON.parse(values.get('test.library')).library.notes[0].title, 'Second');
failKey = 'test.library';
await assert.rejects(write('test', raw(3, 'b', 'Recover me'), 2, false), /Quota/);
assert.equal(JSON.parse(values.get('test.library')).revision, 2);
assert.equal(JSON.parse(values.get('test.recovery')).revision, 3);
failKey = null;
await write('test', raw(3, 'b', 'Recover me'), 3, false);
values.set('test.library', '{broken');
values.set('test.recovery', 'null');
const backup = values.get('test.previous');
await write('test', raw(4, 'd', 'Repaired from previous'), 2, false);
assert.equal(values.get('test.previous'), backup);
values.set('test.recovery', JSON.stringify({format: 99, revision: 5}));
const preserved = values.get('test.library');
await assert.rejects(write('test', raw(5, 'd', 'Must not write'), 4, false), /Unsupported/);
assert.equal(values.get('test.library'), preserved);
delete navigator.locks;
await assert.rejects(read('test'), /Web Locks/);
console.log('Fieldnotes Web storage: serialization, conflicts, quota recovery, previous fallback and future formats passed');
