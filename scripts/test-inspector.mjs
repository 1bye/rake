// Covers: owned runtime state, bounded diagnostics, relationships and safe mutations.
// Real: inspector projection model. Doubles: owned protocol replies; no DOM or transport.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const code = readFileSync(new URL('./inspector-model.js', import.meta.url), 'utf8');
const {InspectorState, commandArguments, matches, request, requests, safeJSON, traces} = await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));
const state = new InspectorState(2, 4096);
state.reset('old');
const snapshot = {runtime_id: 'old', tabs: [{id: 7}], operations: [[9, 'Committed']]};
state.event({kind: 'snapshot', cursor: {runtime_id: 'old', epoch: 1, revision: 2}, snapshot});
assert.equal(state.snapshot, snapshot);
assert.equal(state.event({kind: 'changed', cursor: {runtime_id: 'other', epoch: 1, revision: 3}, snapshot: {runtime_id: 'other'}}), false);
assert.equal(state.snapshot, snapshot);
state.event({kind: 'gap', cursor: {runtime_id: 'old', epoch: 1, revision: 10}, snapshot});
assert.equal(state.gaps, 1);
assert.throws(() => state.event({kind: 'changed', cursor: {runtime_id: 'old', epoch: 1, revision: 11}}));
const record = (sequence, fields = {}) => ({sequence, elapsed_us: sequence * 10, kind: 'event', fields});
state.diagnostics({records: [record(1), record(2), record(3)], dropped: 5, output_dropped: 2}, 'old');
assert.equal(state.evicted, 1);
assert.deepEqual(state.diagnosticsRecords.map(item => item.sequence), [2, 3]);
state.diagnostics({records: [record(3), record(6)], dropped: 7, output_dropped: 2}, 'old');
assert.equal(state.diagnosticGaps, 2);
assert.deepEqual(state.diagnosticsRecords.map(item => item.sequence), [3, 6]);
assert.equal(state.snapshot, snapshot, 'telemetry must never reconstruct authoritative state');
assert.equal(matches(record(1, {task_id: 'task-4', tab_id: 'tab-7'}), {task_id: 4, tab_id: 7}), true);
assert.equal(matches(record(1), {tab_id: 7}), false, 'missing context must not follow selected tab');
const spans = traces([
  {...record(1), kind: 'span_open', span_id: 1, parent_span_id: null, spans: ['same']},
  {...record(2), kind: 'span_open', span_id: 2, parent_span_id: 1, spans: ['same', 'same']},
  {...record(3), kind: 'span_close', span_id: 2, parent_span_id: 1, duration_us: 5, spans: ['same', 'same']},
  {...record(4), kind: 'span_close', span_id: 3, parent_span_id: 99, duration_us: 9, spans: ['lost', 'same']},
]);
assert.equal(spans.length, 3);
assert.equal(spans[1].depth, 1);
assert.equal(spans[2].missingParent, true);
assert.equal(spans[2].opened, false);
const lifecycle = requests([
  {...record(1, {request_id: 1, phase: 'started', method: 'GET', url: 'http://localhost/path'}), kind: 'request'},
  {...record(2, {request_id: 1, phase: 'headers_received', status: 200}), kind: 'request'},
  {...record(3, {request_id: 1, phase: 'abandoned', received_bytes: 8, duration_us: 10}), kind: 'request'},
]);
assert.equal(lifecycle[0].fields.phase, 'abandoned');
assert.equal(lifecycle[0].fields.method, 'GET');
assert.equal(lifecycle[0].started, true);
assert.throws(() => request('old', {type: 'command', value: {type: 'select', tab: Number.MAX_SAFE_INTEGER + 1}}, 'key'));
assert.throws(() => safeJSON('{"id":9007199254740993}'));
const command = {type: 'command', value: {type: 'select', tab: 7}};
const original = request('old', command, 'stable-key');
state.reset('replacement');
assert.equal(state.snapshot, null);
assert.equal(state.diagnosticsRecords.length, 0);
assert.equal(original.runtime_id, 'old');
assert.equal(original.key, 'stable-key');
const bytes = new InspectorState(100, 10); bytes.reset('r');
bytes.diagnostics({records: [record(1, {large: 'too large'})], dropped: 0, output_dropped: 0}, 'r');
assert.equal(bytes.diagnosticsRecords.length, 0);
console.log('Inspector model checks passed');

assert.deepEqual(commandArguments({type: 'object', required: ['id', 'mode'], properties: {
  id: {$ref: '#/$defs/Id'}, mode: {$ref: '#/$defs/Mode'}, limit: {default: 10}, optional: {type: 'string'}},
  $defs: {Id: {type: 'integer', minimum: 0}, Mode: {enum: ['summary', 'detail']}}}), {id: 0, mode: 'summary', limit: 10});
assert.deepEqual(commandArguments({type: 'array', prefixItems: [{type: 'string'}, {type: 'boolean'}]}), ['', false]);
assert.equal(commandArguments({type: 'null'}), null);
assert.equal(commandArguments({$ref: '#/$defs/Loop', $defs: {Loop: {$ref: '#/$defs/Loop'}}}), null);
assert.equal(commandArguments({$ref: 'https://example.invalid/schema'}), null);
assert.deepEqual(commandArguments({default: {a: false}}), {a: false});
