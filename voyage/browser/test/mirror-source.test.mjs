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
  const source = await readFile(join(dirname(fileURLToPath(import.meta.url)), '..', 'mirror-source.mjs'), 'utf8');
  runInNewContext(source, scope);

  const first = scope.__voyageMirror.drain(0);
  assert.equal(first.reset, true);
  assert.equal(first.events.length, 2);
  for (let index = 0; index < 5000; index++) emit({type:3,data:{index}});
  const recovered = scope.__voyageMirror.drain(first.cursor);
  assert.equal(recovered.reset, true);
  assert.deepEqual(Array.from(recovered.events, event => event.type), [4, 2]);
  assert.ok(recovered.cursor <= 1026, 'overflow must stop retaining new mutations');
  assert.equal(scope.__voyageMirror.drain(recovered.cursor).events.length, 0);
});
