import test from 'node:test';
import assert from 'node:assert/strict';
import {voyageList} from '../resources/react/presentation.ts';
const voyage=(id:string,state:string,date='2026-09-01T00:00:00Z')=>({session_id:id,name:id,state:'live',catalogue:{summary:{run_state:state,last_turn_end:date}}});
test('active voyages stay above newer completed voyages across all Vessels',()=>{
 const connections=[{id:'a',name:'A',voyages:[voyage('completed','completed','2026-09-20T00:00:00Z'),voyage('running','running')]},{id:'b',name:'B',voyages:[voyage('waiting','waiting'),voyage('starting','starting'),voyage('blocked','blocked'),voyage('cancelling','cancelling')]}];
 assert.deepEqual(voyageList(connections).map(v=>v.session_id),['running','blocked','cancelling','starting','waiting','completed']);
 assert.equal(voyageList(connections,'completed')[0].session_id,'completed');
});
test('fresh snapshots immediately promote starts and demote finished voyages',()=>{
 const connections=[{id:'a',name:'A',voyages:[voyage('old-run','running','2026-09-20T00:00:00Z'),voyage('new-run','idle')]}];
 const snapshotFor=(_:any,v:any)=>({run:{state:v.session_id==='old-run'?'completed':'running'}});
 assert.deepEqual(voyageList(connections,'',snapshotFor).map(v=>v.session_id),['new-run','old-run']);
});
test('terminal lifecycle and stopped processes are not pinned by old run metadata',()=>{
 const stopped={...voyage('stopped','running'),state:'stopped'};
 const archived=voyage('archived','running');(archived.catalogue.summary as any).archived=true;
 const connections=[{id:'a',name:'A',voyages:[stopped,archived,voyage('active','running')]}];
 assert.deepEqual(voyageList(connections).map(v=>[v.session_id,v.active]),[['active',true],['archived',false],['stopped',false]]);
});
