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

// OS notification adapter: permission UI is doubled, delivery/callback code is real.
const notificationsSource = await readFile(new URL('../crates/rake/src/platform/notifications/browser.js', import.meta.url), 'utf8');
const notifications = await import(`data:text/javascript;base64,${Buffer.from(notificationsSource).toString('base64')}`);
let prompts = 0;
let nextPermission = 'granted';
const posted = [];
class NotificationHost {
  static permission = 'default';
  static requestPermission() {
    prompts++;
    this.permission = nextPermission;
    return Promise.resolve(this.permission);
  }
  constructor(title, options) {
    this.title = title;
    this.options = options;
    this.closed = false;
    posted.push(this);
  }
  close() { this.closed = true; }
}
globalThis.Notification = NotificationHost;
globalThis.isSecureContext = true;
navigator.userActivation.isActive = false;
assert.equal(notifications.notification_permission(), 'default');
assert.equal(prompts, 0, 'query must not prompt');
await assert.rejects(notifications.request_notification_permission(), {name: 'NotAllowedError'});
assert.equal(prompts, 0, 'a user event is required');
navigator.userActivation.isActive = true;
const authorization = notifications.request_notification_permission();
assert.equal(prompts, 1, 'permission starts before awaiting');
assert.equal(await authorization, 'granted');
assert.equal(await notifications.request_notification_permission(), 'granted');
assert.equal(prompts, 1, 'existing decisions are not prompted again');
const activations = [];
let focused = 0;
window.focus = () => { focused++; };
const notificationHost = notifications.create_host(tag => activations.push(tag));
assert.equal(notifications.show_notification(notificationHost, 'export', 'Done 漢字', 'Open export'), true);
const first = posted.at(-1);
assert.equal(first.options.tag, 'export');
assert.equal(first.title, 'Done 漢字');
notifications.show_notification(notificationHost, 'export', 'New export', 'Open replacement');
assert.equal(first.closed, true);
assert.equal(first.onclick, null);
const second = posted.at(-1);
second.onclick();
assert.equal(focused, 1);
assert.deepEqual(activations, ['export']);
assert.equal(second.closed, true);
assert.equal(second.onclick, null);
assert.equal(notificationHost.posted.size, 0);
notifications.show_notification(notificationHost, 'other', 'Done', 'Dismiss me');
const dismissed = posted.at(-1);
notifications.dismiss_notification(notificationHost, 'other');
notifications.dismiss_notification(notificationHost, 'missing');
assert.equal(dismissed.closed, true);
assert.equal(dismissed.onclick, null);
assert.deepEqual(activations, ['export']);
NotificationHost.permission = 'denied';
const before = posted.length;
assert.equal(await notifications.request_notification_permission(), 'denied');
assert.equal(notifications.show_notification(notificationHost, 'denied', 'No', ''), false);
assert.equal(posted.length, before);
assert.equal(prompts, 1);
NotificationHost.permission = 'default';
nextPermission = 'denied';
assert.equal(await notifications.request_notification_permission(), 'denied');
assert.equal(prompts, 2);
NotificationHost.permission = 'granted';
notifications.show_notification(notificationHost, 'shutdown', 'Done', '');
const shutdown = posted.at(-1);
notifications.dispose_host(notificationHost);
assert.equal(shutdown.onclick, null);
assert.equal(shutdown.closed, true);
assert.equal(notificationHost.activate, null);
globalThis.isSecureContext = false;
assert.throws(() => notifications.notification_permission(), {name: 'NotSupportedError'});
await assert.rejects(notifications.request_notification_permission(), {name: 'NotSupportedError'});
globalThis.isSecureContext = true;
delete globalThis.Notification;
assert.throws(() => notifications.notification_permission(), {name: 'NotSupportedError'});
console.log('Notifications: eager authorization, denial, replacement, click focus/activation and disposal passed');
