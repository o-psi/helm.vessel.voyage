import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {dirname, join} from 'node:path';
import {runInNewContext} from 'node:vm';
import {test} from 'node:test';

test('recorder bounds retained mutations and recovers with a full snapshot', async () => {
  let emit;
  const record = options => {
    emit = options.emit;
    emit({type:4,data:{href:'https://example.test'}});
    emit({type:2,data:{node:{type:0,childNodes:[]}}});
    return () => {};
  };
  record.takeFullSnapshot = () => emit({type:2,data:{node:{type:0,childNodes:[]}}});
  const scope = {rrweb:{record}};
  const source = (await readFile(join(dirname(fileURLToPath(import.meta.url)), '..', 'mirror-source.mjs'), 'utf8')).replace('__VOYAGE_CAPTURE_KEY__','fixture-key');
  runInNewContext(source, scope);scope.__voyageMirror.enable('fixture-key',1);

  const first = scope.__voyageMirror.enable('fixture-key',1);scope.__voyageMirror.drain(0,2200000,'fixture-key',1);
  assert.equal(first.reset, true);
  assert.equal(first.events.length, 2);
  for (let index = 0; index < 5000; index++) emit({type:3,data:{index}});
  const recovered = scope.__voyageMirror.drain(first.cursor,2200000,'fixture-key',1);
  assert.equal(recovered.reset, true);
  assert.deepEqual(Array.from(recovered.events, event => event.type), [4, 2]);
  assert.ok(recovered.cursor <= 1026, 'overflow must stop retaining new mutations');
  assert.equal(scope.__voyageMirror.drain(recovered.cursor,2200000,'fixture-key',1).events.length, 0);
});

test('recorder stop acknowledges actual inactive state and clears queued capture',async()=>{
 let ended=0;
 const record=options=>{options.emit({type:2,data:{node:{type:0,childNodes:[]}}});return ()=>{ended++;};};
 record.takeFullSnapshot=()=>{};
 const scope={rrweb:{record}};
 const source=(await readFile(join(dirname(fileURLToPath(import.meta.url)),'..','mirror-source.mjs'),'utf8')).replace('__VOYAGE_CAPTURE_KEY__','fixture-key');
 runInNewContext(source,scope);
 const original=scope.__voyageMirror;
 assert.equal(Object.getOwnPropertyDescriptor(scope,'__voyageMirror').writable,false);
 assert.equal(Object.getOwnPropertyDescriptor(scope,'__voyageMirror').configurable,false);
 assert.throws(()=>Object.defineProperty(scope,'__voyageMirror',{value:{stop:()=>({recording:false})}}));
 runInNewContext(source,scope);assert.equal(scope.__voyageMirror,original);
 scope.__voyageMirror.drain(0,2200000,'fixture-key',1);
 const stopped=scope.__voyageMirror.stop('fixture-key',2);
 assert.equal(ended,1);assert.equal(stopped.recording,false);assert.equal(stopped.pending_events,0);assert.equal(stopped.pending_bytes,0);
 assert.equal(scope.__voyageMirror.enable('wrong',3),false);assert.equal(scope.__voyageMirror.enable('fixture-key',1),false);assert.equal(scope.__voyageMirror.drain(0).error,'recorder_disabled');
 const repeated=scope.__voyageMirror.stop('fixture-key',2);assert.equal(ended,1);assert.equal(repeated.recording,false);
});
