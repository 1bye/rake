// Covers: pending request rejection on browser/transport loss and the unchanged timeout.
// Real: browser-test-cdp request tracking. Doubles: Chrome process and WebSocket events.
// Requires: Node 22+; node --test scripts/test-browser-cdp.mjs. No GUI claims.
import assert from 'node:assert/strict';
import {EventEmitter} from 'node:events';
import {test} from 'node:test';
import {createBrowserCdp} from './browser-test-cdp.mjs';

class Socket extends EventTarget {
  readyState = WebSocket.OPEN;
  sent = [];
  send(value) { this.sent.push(JSON.parse(value)); }
  close() {
    this.readyState = WebSocket.CLOSED;
    this.dispatchEvent(Object.assign(new Event('close'), {code: 1006, reason: ''}));
  }
}

function setup(t) {
  const socket = new Socket();
  const chrome = Object.assign(new EventEmitter(), {exitCode: null, signalCode: null});
  const events = [];
  const client = createBrowserCdp(socket, chrome, {onEvent: event => events.push(event), stderr: () => 'browser diagnostic'});
  t.after(() => client.close());
  return {socket, chrome, events, client};
}

async function rejectedImmediately(calls, pattern) {
  let results;
  Promise.allSettled(calls).then(value => { results = value; });
  await new Promise(resolve => setImmediate(resolve));
  assert.ok(results, 'pending calls must settle before the next event-loop turn, without advancing the request timer');
  for (const result of results) {
    assert.equal(result.status, 'rejected');
    assert.match(result.reason.message, pattern);
  }
}

for (const [code, signal] of [[0, null], [null, 'SIGTERM']]) {
  test(`Chrome exit (${signal ?? code}) rejects all pending calls`, async t => {
    const {client, chrome, socket} = setup(t);
    const calls = [client.call('Page.navigate'), client.call('Runtime.evaluate')];
    chrome.exitCode = code;
    chrome.signalCode = signal;
    chrome.emit('exit', code, signal);
    await rejectedImmediately(calls, /Chrome exited .*browser diagnostic/);
    await rejectedImmediately([client.call('Page.reload')], /Chrome exited/);
    assert.equal(socket.sent.length, 2, 'closed connections must not send or retry calls');
  });
}

test('WebSocket closure rejects every pending call and remains terminal', async t => {
  const {client, socket, chrome} = setup(t);
  const calls = [client.call('Page.navigate'), client.call('Runtime.evaluate')];
  socket.close();
  chrome.emit('exit', 0, null);
  socket.close();
  await rejectedImmediately(calls, /CDP WebSocket closed/);
  await rejectedImmediately([client.call('Page.reload')], /CDP WebSocket closed/);
  assert.equal(socket.sent.length, 2);
});

test('WebSocket errors reject pending calls', async t => {
  const {client, socket} = setup(t);
  const calls = [client.call('Page.navigate'), client.call('Runtime.evaluate')];
  socket.dispatchEvent(new Event('error'));
  await rejectedImmediately(calls, /CDP WebSocket error/);
});

test('an already exited browser or closed socket cannot start a request', async t => {
  const chrome = Object.assign(new EventEmitter(), {exitCode: 0, signalCode: null});
  const socket = new Socket();
  const deadBrowser = createBrowserCdp(socket, chrome);
  t.after(() => deadBrowser.close());
  await rejectedImmediately([deadBrowser.call('Page.navigate')], /Chrome exited/);
  chrome.exitCode = null;
  socket.close();
  const deadSocket = createBrowserCdp(socket, chrome);
  t.after(() => deadSocket.close());
  await rejectedImmediately([deadSocket.call('Page.navigate')], /CDP WebSocket closed/);
  assert.equal(socket.sent.length, 0);
});

test('a send failure rejects pending calls and cleanup removes the exit listener', async t => {
  const {client, socket, chrome} = setup(t);
  const calls = [client.call('Page.navigate')];
  socket.send = () => { throw new Error('send failed'); };
  calls.push(client.call('Runtime.evaluate'));
  await rejectedImmediately(calls, /send failed/);
  client.close();
  client.close();
  assert.equal(chrome.listenerCount('exit'), 0);
  await rejectedImmediately([client.call('Page.reload')], /send failed/);
});

test('replies preserve session IDs, protocol errors and event delivery', async t => {
  const {client, socket, events} = setup(t);
  const first = client.call('Page.navigate', {url: 'http://127.0.0.1/'}, 'session');
  assert.deepEqual(socket.sent[0], {id: 1, method: 'Page.navigate', params: {url: 'http://127.0.0.1/'}, sessionId: 'session'});
  socket.dispatchEvent(new MessageEvent('message', {data: JSON.stringify({id: 1, result: {frameId: 'frame'}})}));
  assert.deepEqual(await first, {frameId: 'frame'});
  const second = client.call('Unknown');
  const rejected = assert.rejects(second, /unknown method/);
  socket.dispatchEvent(new MessageEvent('message', {data: JSON.stringify({id: 2, error: {code: -32601, message: 'unknown method'}})}));
  await rejected;
  const event = {method: 'Page.loadEventFired', sessionId: 'session', params: {timestamp: 1}};
  socket.dispatchEvent(new MessageEvent('message', {data: JSON.stringify(event)}));
  assert.deepEqual(events, [event]);
});

test('the request deadline remains exactly 25 seconds', async t => {
  t.mock.timers.enable({apis: ['setTimeout']});
  const {client} = setup(t);
  let outcome;
  const result = client.call('Page.navigate').catch(error => { outcome = error; });
  t.mock.timers.tick(24_999);
  await Promise.resolve();
  assert.equal(outcome, undefined);
  t.mock.timers.tick(1);
  await result;
  assert.match(outcome.message, /^Page.navigate timed out$/);
});
