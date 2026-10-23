// Covers: actual panel rendering, typed forms, decisions and exact retry envelopes.
// Real: inspector UI module and projections. Doubles: minimal DOM, clock and runtime transport.
// Requires: Node 22+; this does not establish browser layout or GPU execution.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

class Element {
  constructor(tag = 'div') {
    this.tagName = tag; this.children = []; this.dataset = {}; this.attributes = {};
    this.listeners = {}; this.value = ''; this.classList = {add: () => {}};
  }
  set textContent(value) { this.children = []; this.content = String(value); }
  get textContent() { return (this.content || '') + this.children.map(child => child.textContent ?? child).join(''); }
  append(...children) { for (const child of children) { this.children.push(child); if (child instanceof Element) child.parent = this; } }
  replaceChildren(...children) { this.children = []; this.content = ''; this.append(...children); }
  setAttribute(name, value) { this.attributes[name] = String(value); }
  addEventListener(name, callback) { this.listeners[name] = callback; }
  get firstChild() { return this.children[0]; }
  closest(tag) { return this.tagName === tag ? this : this.parent?.closest(tag); }
  click() { return this.listeners.click?.(); }
}
const ids = Object.fromEntries([...readFileSync(new URL('./inspector.html', import.meta.url), 'utf8').matchAll(/id="([^"]+)"/g)].map(([, id]) => [id, new Element()]));
globalThis.Node = Element;
globalThis.document = {getElementById: id => ids[id], createElement: tag => new Element(tag),
  createElementNS: (_namespace, tag) => new Element(tag), createTextNode: value => ({textContent: value}), activeElement: null};
globalThis.location = {search: '', hash: '#__rake_control=' + 'x'.repeat(40), pathname: '/__rake/inspector'};
const storage = new Map();
globalThis.sessionStorage = {setItem: (key, value) => storage.set(key, value), getItem: key => storage.get(key)};
globalThis.history = {state: null, replaceState: (_state, _title, url) => assert.equal(url, '/__rake/inspector')};
const timers = [];
globalThis.setTimeout = (callback, delay) => { const timer = {callback, delay}; timers.push(timer); return timer; };
globalThis.clearTimeout = timer => { timer.cancelled = true; };
const runtime = '2da9d174-653e-4b5e-80ad-1ab0857e8ec1';
const information = {runtime_id: runtime, platform: 'native', supported_targets: ['CurrentTab', 'NewTab'],
  commands: ['open', 'navigate', 'reveal', 'select', 'close', 'back', 'forward', 'mode', 'cancel_task', 'cancel_operation', 'respond', 'run']};
const fields = {task_id: 'task-5', tab_id: 'tab-7', entry_id: 'entry-8', operation_id: 'op-9', request_id: 10};
const diagnostics = [
  {sequence: 1, elapsed_us: 5, kind: 'span_open', span_id: 1, parent_span_id: null, spans: ['task'], fields},
  {sequence: 2, elapsed_us: 10, kind: 'event', level: 'INFO', target: 'example', fields: {...fields, message: '<script>literal</script>'}},
  {sequence: 3, elapsed_us: 15, kind: 'request', fields: {...fields, phase: 'started', method: 'GET', url: 'http://localhost/resource'}},
  {sequence: 4, elapsed_us: 30, kind: 'request', fields: {...fields, phase: 'abandoned', status: 200, received_bytes: 3, duration_us: 15}},
  {sequence: 5, elapsed_us: 40, kind: 'span_close', span_id: 1, parent_span_id: null, duration_us: 35, spans: ['task'], fields},
];
const snapshot = {runtime_id: runtime, window_id: 1, revision: 2, selected: 7,
  reload: {generation: 1, phase: 'idle'},
  tabs: [{id: 7, mode: 'Live', cursor: 1, entries: [
    {id: 6, destination: {route: 'home', params: {}}, dirty: false, revision: 1},
    {id: 8, destination: {route: 'document', params: {id: 'A'}}, dirty: true, revision: 3}]}],
  work: {host_visibility: 'Visible', tasks: [{id: 5, key: 'refresh', kind: 'Activity', owner: {Entry: 8},
    origin: {runtime_id: runtime, tab_id: 7, entry_id: 8}, run_id: 11, generation_id: 1, eligible: true, state: 'Running'}]},
  operations: [[9, 'AwaitingDecision']], decisions: [{id: 12, operation_id: 9, entry_id: 8, message: 'Save changed entry?', choices: [{id: 'save', label: 'Save'}, {id: 'discard', label: 'Discard'}]}]};
const envelopes = [];
let advertised = [{application_id: 'test', info: information, transport: 'local'}];
let loseReply = false;
let expireOperation = false;
globalThis.fetch = async (url, options) => {
  assert.equal(options.headers['X-Rake-Token'], 'x'.repeat(40));
  const body = JSON.parse(options.body);
  let result;
  if (url.endsWith('inspect_instances')) result = advertised;
  else if (url.endsWith('development')) result = {phase: 'watching', published: 1};
  else if (url.endsWith('inspect_output')) result = {records: [], dropped: 0};
  else {
    const envelope = body.request;
    assert.equal(envelope.runtime_id, runtime);
    if (envelope.message.type === 'command') {
      envelopes.push(envelope);
      if (loseReply) { loseReply = false; throw new Error('response lost'); }
      result = {Ok: {type: 'submitted', value: {operation_id: 9}}};
    } else {
      const query = envelope.message.value;
      const replies = {info: information, routes: [{id: 'document', path: '/documents/{id}', params: [{name: 'id', value_type: 'String', required: true}]}],
        commands: [{id: 'test.echo', capability: 'Headless', arguments: {properties: {message: {type: 'string'}}}, result: {type: 'string'}}],
        events: {cursor: {runtime_id: runtime, epoch: 1, revision: 2}, kind: 'snapshot', snapshot},
        diagnostics: {records: diagnostics.filter(record => record.sequence > query.after), dropped: 0, output_dropped: 0},
        operation: {operation_id: 9, state: 'AwaitingDecision', result: null}};
      result = {Ok: {type: 'data', value: replies[query.type]}};
      if (query.type === 'operation' && expireOperation) result = {Err: {OperationExpired: query.operation}};
    }
  }
  return {ok: true, json: async () => structuredClone(result)};
};
async function settle() { for (let index = 0; index < 40; index++) await Promise.resolve(); }
function all(node) { return [node, ...node.children.flatMap(child => child instanceof Element ? all(child) : [])]; }
function find(tag, text) { const node = all(ids.content).find(item => item.tagName === tag && item.textContent.includes(text)); assert.ok(node, `${tag}: ${text}`); return node; }
async function panel(name) { const button = ids.panels.children.find(item => item.dataset.panel === name); await button.click(); await settle(); assert.equal(ids.title.textContent, name); }
const model = 'data:text/javascript;base64,' + Buffer.from(readFileSync(new URL('./inspector-model.js', import.meta.url))).toString('base64');
const code = readFileSync(new URL('./inspector.js', import.meta.url), 'utf8').replace("'/__rake/inspector-model.js'", JSON.stringify(model));
await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));
await settle();
await timers.find(timer => timer.delay === 750).callback(); await settle();
assert.match(ids.content.textContent, /Current/);
assert.match(ids.content.textContent, /Dirty/);

await panel('Routes');
const input = all(ids.content).find(item => item.attributes['aria-label'] === 'document id');
input.value = 'B'; input.listeners.input();
const open = find('button', 'Open');
open.closest('form').listeners.submit({preventDefault() {}, submitter: open}); await settle();
assert.deepEqual(envelopes.at(-1).message.value, {type: 'open', destination: {route: 'document', params: {id: 'B'}}, options: {target: 'NewTab', mode: null, activation: 'Activate'}});

await panel('Tasks'); assert.match(ids.content.textContent, /entry-8/);
await find('button', 'Cancel task').click(); await settle();
assert.deepEqual(envelopes.at(-1).message.value, {type: 'cancel_task', task: 5});
await panel('Operations'); assert.match(ids.content.textContent, /AwaitingDecision/);
expireOperation = true;
await find('button', 'Inspect result').click(); await settle();
assert.match(ids.content.textContent, /OperationExpired/);
assert.match(ids.connection.textContent, /Connected/);
expireOperation = false;
await panel('Decisions'); assert.match(ids.content.textContent, /Save changed entry/);
await find('button', 'Save').click(); await settle();
assert.deepEqual(envelopes.at(-1).message.value, {type: 'respond', decision: 12, choice: 'save'});
await panel('Console'); assert.match(ids.content.textContent, /<script>literal<\/script>/);
const argumentsInput = all(ids.content).find(item => item.tagName === 'textarea');
argumentsInput.value = '{"message":"hello"}'; argumentsInput.listeners.input();
const run = find('button', 'Run command');
loseReply = true;
run.closest('form').listeners.submit({preventDefault() {}}); await settle();
const uncertain = envelopes.at(-1);
await find('button', 'Retry original request').click(); await settle();
assert.deepEqual(envelopes.at(-1), uncertain);
assert.deepEqual(uncertain.message.value, {type: 'run', id: 'test.echo', arguments: {message: 'hello'}});
loseReply = true;
find('button', 'Run command').closest('form').listeners.submit({preventDefault() {}}); await settle();
await panel('Requests'); assert.match(ids.content.textContent, /abandoned/);
assert.match(ids.content.textContent, /localhost\/resource/);
await panel('Traces'); assert.match(ids.content.textContent, /Closed/);
assert.ok(all(ids.content).some(node => node.tagName === 'svg'));
await panel('Reload'); assert.match(ids.content.textContent, /watching/);
const successor = '3da9d174-653e-4b5e-80ad-1ab0857e8ec1';
advertised = [{application_id: 'test', info: {...information, runtime_id: successor}}];
await panel('Reload');
assert.equal(ids.runtime.value, ''); // Another instance of the same app is not a proven successor.
advertised[0].restored_from = runtime;
await panel('Reload');
assert.equal(ids.runtime.value, successor);
assert.match(ids.content.textContent, /previous runtime/);
assert.equal(find('button', 'Retry original request').disabled, true);
console.log('Inspector panel and command checks passed');
