// Covers: actual panel rendering, typed forms, decisions and exact retry envelopes.
// Real: inspector UI module and projections. Doubles: minimal DOM, clock and runtime transport.
// Requires: Node 22+; this does not establish browser layout or GPU execution.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';

class Element {
  constructor(tag = 'div') {
    this.tagName = tag; this.children = []; this.dataset = {}; this.attributes = {};
    this.listeners = {}; this.value = ''; this.style = {setProperty(name, value) { this[name] = String(value); }};
    this.classList = {
      add: (...values) => this.className = [...new Set([...(this.className || '').split(' ').filter(Boolean), ...values])].join(' '),
      contains: value => (this.className || '').split(' ').includes(value),
    };
  }
  set textContent(value) { this.children = []; this.content = String(value); }
  get textContent() { return (this.content || '') + this.children.map(child => child.textContent ?? child).join(''); }
  append(...children) { for (const child of children) { this.children.push(child); if (child instanceof Element) child.parent = this; } }
  replaceChildren(...children) { this.children = []; this.content = ''; this.append(...children); }
  setAttribute(name, value) { this.attributes[name] = String(value); }
  removeAttribute(name) { delete this.attributes[name]; }
  addEventListener(name, callback) { this.listeners[name] = callback; }
  get firstChild() { return this.children[0]; }
  closest(tag) { return this.tagName === tag ? this : this.parent?.closest(tag); }
  click() { return this.listeners.click?.(); }
  focus() { document.activeElement = this; }
  querySelectorAll(selector) {
    assert.ok(['[data-scroll]', '[data-focus]'].includes(selector));
    const key = selector === '[data-scroll]' ? 'scroll' : 'focus';
    return this.children.flatMap(child => child instanceof Element ? [...(key in child.dataset ? [child] : []), ...child.querySelectorAll(selector)] : []);
  }
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
const information = {element_interaction: true, runtime_id: runtime, platform: 'native', supported_targets: ['CurrentTab', 'NewTab', 'NewWindow'],
  queries: ['inspect_tree', 'inspect_element'],
  commands: ['quit', 'close_window', 'open', 'navigate', 'reveal', 'select', 'close', 'back', 'forward', 'mode', 'cancel_task', 'cancel_operation', 'respond', 'run']};
const fields = {task_id: 'task-5', tab_id: 'tab-7', entry_id: 'entry-8', operation_id: 'op-9', request_id: 10};
const diagnostics = [
  {sequence: 1, elapsed_us: 5, kind: 'span_open', span_id: 1, parent_span_id: null, spans: ['task'], fields},
  {sequence: 2, elapsed_us: 10, kind: 'event', level: 'INFO', target: 'example', fields: {...fields, message: '<script>literal</script>'}},
  {sequence: 3, elapsed_us: 15, kind: 'request', fields: {...fields, phase: 'started', method: 'GET', url: 'http://localhost/resource'}},
  {sequence: 4, elapsed_us: 30, kind: 'request', fields: {...fields, phase: 'abandoned', status: 200, received_bytes: 3, duration_us: 15}},
  {sequence: 5, elapsed_us: 40, kind: 'span_close', span_id: 1, parent_span_id: null, duration_us: 35, spans: ['task'], fields},
];
const snapshot = {runtime_id: runtime, window_id: 1, revision: 2, selected: 7,
  quit: {phase: 'idle', operation_id: null, options: {jobs: 'wait'}, pending_tasks: [], pending_operations: []},
  reload: {generation: 1, phase: 'idle'},
  tabs: [{id: 7, mode: 'Live', cursor: 1, entries: [
    {id: 6, destination: {route: 'home', params: {}}, dirty: false, revision: 1},
    {id: 8, destination: {route: 'document', params: {id: 'A'}}, dirty: true, revision: 3}]}],
  work: {host_visibility: 'Visible', tasks: [{id: 5, key: 'refresh', kind: 'Activity', owner: {Entry: 8},
    origin: {runtime_id: runtime, tab_id: 7, entry_id: 8}, run_id: 11, generation_id: 1, eligible: true, state: 'Running'}]},
  operations: [[9, 'AwaitingDecision']], decisions: [{id: 12, operation_id: 9, entry_id: 8, message: 'Save changed entry?', choices: [{id: 'save', label: 'Save'}, {id: 'discard', label: 'Discard'}]}]};
let elementCapture = {status: 'captured', capture: '1', router_revision: 2, omitted: 3, coverage: 'Inspection boundaries; opaque GPUI children unavailable.', elements: [
  {id: 'element-1-0', parent: null, semantic_id: '<script>screen</script>', kind: 'gpui::Div', tab_id: 7, entry_id: 8,
    layout: {x: 10, y: 20, width: 300, height: 50}, source: {file: 'src/example.rs', line: 12, column: 3}, styles: {display: 'Flex'}, metadata_truncated: false}]};
let staleElement = false;
let pickerMode = false, pickedBoundary = null, pickSequence = 0;
const envelopes = [];
let advertised = [{application_id: 'test', info: information, transport: 'local'}];
let loseReply = false;
let expireOperation = false;
globalThis.fetch = async (url, options) => {
  assert.equal(options.headers['X-Rake-Token'], 'x'.repeat(40));
  const body = JSON.parse(options.body);
  let result;
  if (url.endsWith('inspect_instances')) result = advertised;
  else if (url.endsWith('inspect_cdp')) result = [{id: runtime, devtoolsFrontendUrl: `devtools://devtools/bundled/devtools_app.html?ws=127.0.0.1:9229/${'a'.repeat(32)}/${runtime}`}];
  else if (url.endsWith('development')) result = {phase: 'watching', published: 1};
  else if (url.endsWith('inspect_output')) result = {records: [], dropped: 0};
  else {
    const envelope = body.request;
    assert.equal(envelope.runtime_id, runtime);
    if (envelope.message.type === 'command') {
      envelopes.push(envelope);
      if (loseReply) { loseReply = false; throw new Error('response lost'); }
      if (envelope.message.value.type === 'inspect_mode') {
        pickerMode = envelope.message.value.enabled;
        result = {Ok: {type: 'acknowledged'}};
      } else if (envelope.message.value.type === 'highlight_element') result = {Ok: {type: 'acknowledged'}};
      else result = {Ok: {type: 'submitted', value: {operation_id: 9}}};
    } else {
      const query = envelope.message.value;
      const replies = {inspect_pick: {mode: pickerMode, generation: elementCapture.capture, hovered: null, picked: pickedBoundary, pick_sequence: pickSequence}, inspect_tree: elementCapture, inspect_element: elementCapture.elements[0], info: information, routes: [{id: 'document', path: '/documents/{id}', params: [{name: 'id', value_type: 'String', required: true}]}],
        commands: [{id: 'test.echo', capability: 'Headless', arguments: {type: 'object', required: ['message'], properties: {message: {type: 'string'}}}, result: {type: 'string'}},
          {id: 'test.unit', capability: 'RequiresWindow', arguments: {type: 'null'}, result: {type: 'null'}},
          {id: 'test.tuple', capability: 'Headless', arguments: {type: 'array', prefixItems: [{type: 'string'}, {type: 'integer', minimum: 0}]}, result: {type: 'array'}}],
        events: {cursor: {runtime_id: runtime, epoch: 1, revision: snapshot.revision}, kind: 'snapshot', snapshot},
        diagnostics: {records: diagnostics.filter(record => record.sequence > query.after), dropped: 0, output_dropped: 0},
        operation: {operation_id: 9, state: 'AwaitingDecision', result: null}};
      result = {Ok: {type: 'data', value: replies[query.type]}};
      if (query.type === 'inspect_element' && staleElement) result = {Err: {Inspection: 'NotRetained'}};
      if (query.type === 'operation' && expireOperation) result = {Err: {OperationExpired: query.operation}};
    }
  }
  return {ok: true, json: async () => structuredClone(result)};
};
async function settle() { for (let index = 0; index < 40; index++) await Promise.resolve(); }
function all(node) { return [node, ...node.children.flatMap(child => child instanceof Element ? all(child) : [])]; }
function find(tag, text) { const node = all(ids.content).find(item => item.tagName === tag && (item.textContent.includes(text) || item.attributes['aria-label']?.includes(text))); assert.ok(node, `${tag}: ${text}`); return node; }
async function panel(name) { const button = ids.panels.children.find(item => item.dataset.panel === name); await button.click(); await settle(); assert.equal(ids.title.textContent, name); }
const model = 'data:text/javascript;base64,' + Buffer.from(readFileSync(new URL('./inspector-model.js', import.meta.url))).toString('base64');
const code = readFileSync(new URL('./inspector.js', import.meta.url), 'utf8').replace("'/__rake/inspector-model.js'", JSON.stringify(model));
await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));
await settle();
await timers.find(timer => timer.delay === 750).callback(); await settle();
assert.match(ids.content.textContent, /Current/);
assert.equal(ids['open-cdp'].hidden, false);
assert.match(ids['open-cdp'].href, /^devtools:\/\/devtools\/bundled\/devtools_app\.html/);
assert.match(ids.content.textContent, /Dirty/);
assert.equal(ids.panels.children.filter(item => item.tabIndex === 0).length, 1);
assert.equal(ids.panels.children[0].attributes.role, 'tab');
assert.equal(ids.panels.children[0].attributes['aria-selected'], 'true');
let prevented = false;
ids.panels.listeners.keydown({key: 'ArrowRight', preventDefault() { prevented = true; }}); await settle();
assert.ok(prevented);
assert.equal(ids.title.textContent, 'Routes');
assert.equal(document.activeElement.dataset.panel, 'Routes');
ids.panels.listeners.keydown({key: 'Home', preventDefault() {}}); await settle();
assert.equal(ids.title.textContent, 'Workspace');
await ids.theme.click();
assert.equal(ids.app.dataset.theme, 'light');
assert.equal(ids.theme.attributes['aria-label'], 'Switch to dark theme');
assert.equal(storage.get('rake-inspector-theme'), 'light');
await ids.theme.click();
assert.equal(ids.app.dataset.theme, 'dark');
const entryInspector = find('button', 'Inspect entry-8');
entryInspector.focus(); await entryInspector.click(); await settle();
assert.equal(document.activeElement.dataset.focus, 'workspace:entry:8');
const detailPane = all(ids.content).find(item => item.dataset.scroll === 'workspace:detail');
detailPane.scrollTop = 137; detailPane.listeners.scroll();
snapshot.revision++;
await panel('Workspace');
assert.equal(all(ids.content).find(item => item.dataset.scroll === 'workspace:detail').scrollTop, 137);

await find('button', 'Quit (wait for jobs)').click(); await settle();
assert.deepEqual(envelopes.at(-1).message.value, {type: 'quit', options: {jobs: 'wait'}});
await find('button', 'Quit and cancel jobs').click(); await settle();
assert.deepEqual(envelopes.at(-1).message.value, {type: 'quit', options: {jobs: 'cancel'}});
snapshot.revision++;
snapshot.quit = {...snapshot.quit, phase: 'waiting_for_jobs', operation_id: 9, pending_tasks: [5]};
await panel('Workspace');
assert.match(ids.content.textContent, /1 finite jobs/);
await find('button', 'Cancel quit').click(); await settle();
assert.deepEqual(envelopes.at(-1).message.value, {type: 'cancel_operation', operation: 9});
snapshot.revision++;
snapshot.quit = {...snapshot.quit, phase: 'stopping'};
await panel('Workspace');
assert.ok(!all(ids.content).some(item => item.tagName === 'button' && item.textContent === 'Cancel quit'));
snapshot.revision++;
snapshot.quit = {...snapshot.quit, phase: 'ready'};
await panel('Workspace');
assert.ok(!all(ids.content).some(item => item.tagName === 'button' && item.textContent.startsWith('Quit ')));
await panel('Routes');
const input = all(ids.content).find(item => item.attributes['aria-label'] === 'document id');
input.value = 'B'; input.listeners.input();
const open = find('button', 'Open');
open.closest('form').listeners.submit({preventDefault() {}, submitter: open}); await settle();
assert.deepEqual(envelopes.at(-1).message.value, {type: 'open', window: 1, destination: {route: 'document', params: {id: 'B'}}, options: {target: 'NewTab', mode: null, activation: 'Activate'}});

snapshot.windows = [{id: 1, selected: 7, visibility: 'Visible', closing: null}, {id: 4, selected: 14, visibility: 'Hidden', closing: null}];
snapshot.tabs[0].window_id = 1;
snapshot.tabs.push({id: 14, window_id: 4, mode: 'Foreground', cursor: 0, entries: [{id: 15, destination: {route: 'home', params: {}}, dirty: false, revision: 1}]});
snapshot.revision++;
await panel('Workspace');
await find('button', 'Inspect window-4').click(); await settle();
const secondHost = find('h2', 'window-4').parent;
await all(secondHost).find(item => item.tagName === 'button' && item.textContent === 'Close window').click(); await settle();
assert.deepEqual(envelopes.at(-1).message.value, {type: 'close_window', window: 4});
snapshot.windows = [snapshot.windows[1]];
snapshot.revision++;
await panel('Routes');
const hostSelector = all(ids.content).find(item => item.tagName === 'label' && item.textContent.includes('Window for Open')).children.find(item => item.tagName === 'select');
assert.equal(hostSelector.value, '4', 'fall back to a remaining host when the initial window closes');
hostSelector.value = '4'; hostSelector.listeners.change();
const scopeSelector = all(ids.content).find(item => item.tagName === 'label' && item.textContent.includes('Reveal scope')).children.find(item => item.tagName === 'select');
scopeSelector.value = 'Window'; scopeSelector.listeners.change();
const reveal = find('button', 'Reveal');
reveal.closest('form').listeners.submit({preventDefault() {}, submitter: reveal}); await settle();
assert.deepEqual(envelopes.at(-1).message.value, {type: 'reveal', window: 4, scope: 'Window', destination: {route: 'document', params: {id: 'B'}}});
snapshot.windows = [];
snapshot.revision++;
await panel('Routes');
const targetSelector = all(ids.content).find(item => item.tagName === 'label' && item.textContent.includes('Open target')).children.find(item => item.tagName === 'select');
targetSelector.value = 'NewWindow'; targetSelector.listeners.change();
const reopen = find('button', 'Open');
reopen.closest('form').listeners.submit({preventDefault() {}, submitter: reopen}); await settle();
assert.equal(envelopes.at(-1).message.value.window, null);
assert.equal(envelopes.at(-1).message.value.options.target, 'NewWindow');

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
for (const [id, argumentsValue] of [['test.unit', null], ['test.tuple', ['', 0]]]) {
  const editor = all(ids.content).find(item => item.attributes['aria-label'] === id + ' arguments');
  assert.deepEqual(JSON.parse(editor.value), argumentsValue);
  editor.closest('form').listeners.submit({preventDefault() {}}); await settle();
  assert.deepEqual(envelopes.at(-1).message.value, {type: 'run', id, arguments: argumentsValue});
}
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
await panel('Elements'); assert.match(ids.content.textContent, /3 omitted/);
await find('button', 'Pick element in app').click(); await settle();
assert.equal(pickerMode, true);
assert.equal(envelopes.at(-1).key, null);
await timers.find(timer => timer.delay === 100).callback(); await settle();
assert.equal(envelopes.at(-1).message.value.type, 'inspect_mode');
assert.equal(envelopes.at(-1).message.value.lease_ms, 1000);
pickerMode = false; pickedBoundary = 'element-1-0'; pickSequence = 1;
await timers.filter(timer => timer.delay === 100).at(-1).callback(); await settle();
assert.equal(find('button', 'Inspect element-1-0').attributes['aria-pressed'], 'true');
find('button', 'Inspect element-1-0').listeners.pointerenter();
await timers.filter(timer => timer.delay === 100).at(-1).callback(); await settle();
assert.deepEqual(envelopes.at(-1).message.value, {type: 'highlight_element', id: 'element-1-0', generation: '1'});
find('button', 'Inspect element-1-0').listeners.pointerleave();
await timers.filter(timer => timer.delay === 100).at(-1).callback(); await settle();
assert.equal(envelopes.at(-1).message.value.id, null);

assert.match(ids.content.textContent, /<script>screen<\/script>/);
assert.ok(!all(ids.content).some(node => node.tagName === 'script'));
await find('button', 'Inspect element-1-0').click(); await settle();
assert.match(ids.content.textContent, /src\/example.rs:12:3/);
assert.match(ids.content.textContent, /width 300/);
assert.match(ids.content.textContent, /Flex/);
elementCapture.elements[0].resolved = {properties: {background: '#365849'}, hovered: true};
await panel('Elements');
await find('button', 'Computed').click(); await settle();
assert.match(ids.content.textContent, /Resolved GPUI styles · hovered/);
assert.match(ids.content.textContent, /#365849/);
elementCapture.elements[0].resolved = {properties: {background: '#29424b'}, hovered: false};
await panel('Elements');
assert.match(ids.content.textContent, /#29424b/);
assert.equal(find('button', 'Inspect element-1-0').attributes['aria-pressed'], 'true');
await find('button', 'Styles').click(); await settle();
elementCapture = {...elementCapture, capture: '2', elements: [{...elementCapture.elements[0], id: 'element-2-0', styles: null, source: null, layout: null}]};
await panel('Elements'); assert.match(ids.content.textContent, /Details are frozen/);
staleElement = true;
await find('button', 'Inspect element-2-0').click(); await settle();
assert.match(ids.content.textContent, /NotRetained/);
staleElement = false;
await find('button', 'Inspect element-2-0').click(); await settle();
assert.match(ids.content.textContent, /Style refinements unavailable/);
assert.match(ids.content.textContent, /Source location unavailable/);
elementCapture = {...elementCapture, status: 'unavailable', elements: []};
await panel('Elements'); assert.match(ids.content.textContent, /unavailable/);
snapshot.windows = [{id: 4, selected: 1000, visibility: 'Visible', closing: null}];
snapshot.tabs = Array.from({length: 120}, (_, index) => ({id: 1000 + index, window_id: 4, mode: 'Foreground', cursor: 0,
  entries: [{id: 2000 + index, destination: {route: 'document', params: {id: String(index)}}, dirty: false, revision: 1}]}));
snapshot.revision++;
await panel('Workspace');
assert.equal(all(ids.content).filter(item => item.dataset.focus?.startsWith('workspace:')).length, 100, 'bound the rendered workspace tree');
await find('button', 'Next').click(); await settle();
assert.ok(find('button', 'Inspect tab-1050'));
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
