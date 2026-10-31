// Executes the actual worker source against the same fixtures as the Rust model tests.
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import vm from 'node:vm';

const source = await readFile(new URL('../examples/files/web/text.worker.js', import.meta.url), 'utf8');
const cases = JSON.parse(await readFile(new URL('../examples/files/tests/processing.json', import.meta.url), 'utf8'));
let result;
const self = {postMessage(raw) { result = JSON.parse(raw); }};
vm.runInNewContext(source, {self, TextEncoder});
for (const fixture of cases) {
  self.onmessage({data: JSON.stringify({text: fixture.input, normalize: fixture.normalize, trim: fixture.trim})});
  assert.deepEqual(result, fixture.expected);
}
console.log(`Text Toolkit worker: ${cases.length} shared Unicode/normalization fixtures passed`);
