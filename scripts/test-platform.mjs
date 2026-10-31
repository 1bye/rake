// Real browser adapter code with a deterministic DOM/clipboard/storage host.
// Checks boundary behavior, not GPU rendering or browser permission UI.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';

const source = await readFile(new URL('../crates/rake/src/platform/browser.js', import.meta.url), 'utf8');
const adapter = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const values = new Map();
globalThis.localStorage = {
  getItem: key => values.get(key) ?? null,
  setItem(key, value) {
    if (value === 'too big') throw new DOMException('quota', 'QuotaExceededError');
    values.set(key, value);
  },
  removeItem: key => values.delete(key),
};
let clipboard = '';
let initiated = false;
const navigator = {userActivation: {isActive: true}, clipboard: {
  readText() { initiated = true; return Promise.resolve(clipboard); },
  writeText(text) { clipboard = text; return Promise.resolve(); },
}};
Object.defineProperty(globalThis, 'navigator', {value: navigator, configurable: true});
globalThis.isSecureContext = true;
const elements = [];
let choose = 'change';
globalThis.document = {
  body: {append(element) { elements.push(element); }},
  createElement(tag) {
    const element = new EventTarget();
    element.files = [new File(['hello'], 'hello.txt')];
    element.remove = () => elements.splice(elements.indexOf(element), 1);
    element.click = () => {
      assert.equal(tag, 'input');
      queueMicrotask(() => element.dispatchEvent(new Event(choose)));
    };
    return element;
  },
};
assert.equal(adapter.capabilities(), 63);
const pending = adapter.read_clipboard();
assert.equal(initiated, true, 'clipboard must begin in the user event');
assert.equal(await pending, '');
await adapter.write_clipboard('Unicode 漢字');
assert.equal(await adapter.read_clipboard(), 'Unicode 漢字');
navigator.userActivation.isActive = false;
await assert.rejects(adapter.read_clipboard(), {name: 'NotAllowedError'});
await assert.rejects(adapter.pick_file(), {name: 'NotAllowedError'});
navigator.userActivation.isActive = true;
globalThis.isSecureContext = false;
assert.equal(adapter.capabilities() & 12, 0);
await assert.rejects(adapter.read_clipboard(), {name: 'NotSupportedError'});
globalThis.isSecureContext = true;
assert.equal(await adapter.clipboard_permission(false), 'unknown');
navigator.permissions = {query: async () => ({state: 'denied'})};
assert.equal(await adapter.clipboard_permission(false), 'denied');
const file = await adapter.pick_file();
assert.equal(file.name, 'hello.txt');
assert.equal(new TextDecoder().decode(await adapter.read_file(file, 5)), 'hello');
await assert.rejects(adapter.read_file(file, 4), {name: 'RakeSizeLimit'});
let read = false;
await assert.rejects(adapter.read_file({size: 6, arrayBuffer() { read = true; }}, 5), {name: 'RakeSizeLimit'});
assert.equal(read, false);
assert.equal(elements.length, 0);
choose = 'cancel';
assert.equal(await adapter.pick_file(), null);
assert.equal(elements.length, 0);
adapter.preference_set('first', 'theme', 'dark');
adapter.preference_set('second', 'theme', 'light');
assert.equal(adapter.preference_get('first', 'theme'), 'dark');
assert.equal(adapter.preference_get('second', 'theme'), 'light');
assert.throws(() => adapter.preference_set('first', 'theme', 'too big'), {name: 'QuotaExceededError'});
assert.equal(adapter.preference_get('first', 'theme'), 'dark');
adapter.preference_set('first', 'theme', null);
assert.equal(adapter.preference_get('first', 'theme'), null);
Object.defineProperty(globalThis, 'localStorage', {get() { throw new DOMException('blocked', 'SecurityError'); }, configurable: true});
assert.equal(adapter.capabilities() & 16, 0);
assert.throws(() => adapter.preference_get('first', 'theme'), {name: 'SecurityError'});
globalThis.window = {open: () => null};
assert.throws(() => adapter.open_url('https://example.com'), {name: 'NotAllowedError'});
const opened = {opener: {}, location: {href: ''}};
window.open = () => opened;
adapter.open_url('https://example.com');
assert.equal(opened.opener, null);
assert.equal(opened.location.href, 'https://example.com');
console.log('Platform adapter: eager requests, capabilities, denial, cancellation, file limits, quotas and URL opening passed');
