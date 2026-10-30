// Real framing/coordinates/scroll accumulation. No native GPU/browser claims.
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
const code = readFileSync(new URL('./preview-stream.js', import.meta.url),'utf8');
const {Packets, position, Wheel} = await import('data:text/javascript;base64,' + Buffer.from(code).toString('base64'));
function packet(metadata, payload) {
  const h = Buffer.from(JSON.stringify(metadata)), header = Buffer.alloc(8);
  header.writeUInt32BE(h.length); header.writeUInt32BE(payload.length, 4);
  return Buffer.concat([header, h, payload]);
}
const payload = Buffer.alloc(180000, 123);
const binary = Buffer.concat([packet({kind:'config'}, Buffer.from([1,2,3,4])), packet({kind:'frame',key:true}, payload), packet({kind:'frame',key:false}, Buffer.from([9]))]);
for (const step of [1, 7, 65536, binary.length]) {
  const parser = new Packets(), results = [];
  for (let i=0;i<binary.length;i+=step) results.push(...parser.push(binary.subarray(i,i+step)));
  assert.equal(results.length,3); assert.deepEqual(Buffer.from(results[1].payload), payload);
  assert.equal(parser.start,0); assert.equal(parser.end,0);
}
for (const sizes of [[0,1],[4097,1],[1,2*1024*1024+1]]) {
  const header=Buffer.alloc(8);header.writeUInt32BE(sizes[0]);header.writeUInt32BE(sizes[1],4);
  assert.throws(()=>new Packets().push(header), /size/);
}
const window = {frame:{width:760,height:632},content:{x:0,y:32,width:760,height:600}};
assert.deepEqual(position({clientX:50,clientY:66},{left:0,top:0,width:380,height:316},window),{x:100,y:100});
assert.equal(position({clientX:50,clientY:10},{left:0,top:0,width:380,height:316},window),null);
const wheel = new Wheel();
for (let i=0;i<100;i++) wheel.add(.001,.003);
let input=wheel.take();assert.equal(input.dx,6);assert.equal(input.dy,19);assert.equal(input.phase,'started');
wheel.add(.5,1);input=wheel.take();assert.equal(input.dx,32);assert.equal(input.dy,64);assert.equal(input.phase,'moved');
assert.equal(wheel.take(true).phase,'ended');assert.equal(wheel.take(true),null);
wheel.add(10000,-10000);input=wheel.take();assert.equal(input.dx,128000);assert.equal(input.dy,-128000);
console.log('Preview regressions passed: fragmented binary stream, bounded parser, scaled coordinates and fractional scroll phases');
