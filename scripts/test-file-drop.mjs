import assert from 'node:assert/strict';
import {on_canvas_file_drop, read_file} from '../crates/rake/src/platform/browser.js';
globalThis.document = new EventTarget();
const canvas = new EventTarget();
canvas.tagName = 'CANVAS';
canvas.style = {};
const results = [];
const dispose = on_canvas_file_drop(file => results.push(file));
const file = new File(['hello'], 'hello.txt');
function drop(files) {
  const event = new Event('drop', {cancelable: true});
  Object.defineProperties(event, {target: {value: canvas}, dataTransfer: {value: {files}}});
  document.dispatchEvent(event);
  assert(event.defaultPrevented);
}
drop([file]);
assert.equal(results[0], file);
assert.equal(new TextDecoder().decode(await read_file(results[0], 5)), 'hello');
await assert.rejects(read_file(file, 4), {name: 'RakeSizeLimit'});
drop([file, file]);
assert.equal(results[1], null);
dispose();
const event = new Event('drop');
Object.defineProperties(event, {target: {value: canvas}, dataTransfer: {value: {files: [file]}}});
document.dispatchEvent(event);
assert.equal(results.length, 2);
console.log('Canvas drop identity, bounds, count and disposal passed');
