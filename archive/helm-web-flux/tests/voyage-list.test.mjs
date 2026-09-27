import test from 'node:test';
import assert from 'node:assert/strict';
import {voyageActivity, activityLabel, voyageList} from '../resources/js/voyage-list.js';

const entry = (id, last_turn_end, created_at) => ({session_id:id, name:`Voyage ${id}`, catalogue:{summary:{last_turn_end,created_at}}});
test('catalogue retains all entries across Vessels and searches the complete list', () => {
    const connections = [{id:'a',name:'First',voyages:Array.from({length:250},(_,i)=>entry(String(i)))}, {id:'b',name:'Second',voyages:[entry('0')]}];
    assert.equal(voyageList(connections).length,251);
    assert.equal(voyageList(connections,'Voyage 249').length,1);
    assert.equal(voyageList(connections,' SECOND ')[0].connection.id,'b');
    assert.equal(voyageList(connections,'no match').length,0);
});
test('activity uses turn completion then creation, never catalogue observation time', () => {
    const voyage=entry('a','2026-09-15T12:00:00Z','2026-09-01T12:00:00Z');
    assert.equal(voyageActivity(voyage).kind,'Last turn ended');
    voyage.catalogue.summary.last_turn_end='invalid';
    assert.equal(voyageActivity(voyage).kind,'Created');
    voyage.catalogue.summary.created_at=null;
    voyage.catalogue.observed_at_ms=Date.now();
    assert.equal(voyageActivity(voyage),null);
    assert.equal(activityLabel(null),'No timestamp');
});
test('newest voyages sort first with deterministic ties and unknown dates last', () => {
    const connections=[{id:'a',name:'Host',voyages:[entry('unknown'),entry('old',null,'2025-01-01T00:00:00Z'),entry('new','2026-09-15T00:00:00Z'),entry('tie','2026-09-15T00:00:00Z')]}];
    assert.deepEqual(voyageList(connections).map(v=>v.session_id),['new','tie','old','unknown']);
    const date=new Date(2026,8,15,14,30);
    assert.equal(activityLabel({ms:date.getTime()},date),date.toLocaleTimeString(undefined,{hour:'numeric',minute:'2-digit'}));
    assert.match(activityLabel({ms:new Date(2025,0,1).getTime()},date),/2025/);
});
