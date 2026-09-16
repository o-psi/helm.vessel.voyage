import test from 'node:test';
import assert from 'node:assert/strict';
import {actionDescription,actionStatus,actionDuration} from '../resources/js/tool-presentation.js';
test('TUI action facts: descriptions, uncertain state, outcomes and durations',()=>{
 assert.equal(actionDescription({name:'read_file',arguments:{path:'src/main.rs'}}),'Read src/main.rs');
 assert.equal(actionDescription({name:'apply_patch',arguments:{path:'a',patch:'--- a\n+++ a\n-old\n+new'}}),'Edit a · +1/−1 lines');
 assert.equal(actionStatus({},null,false,false),'Unconfirmed');
 assert.equal(actionStatus({},null,true,true),'Awaiting approval');
 assert.equal(actionStatus({}, {tool_outcome:{execution:'policy_refused'}},false,false),'Refused');
 assert.equal(actionStatus({}, {tool_outcome:{execution:'succeeded',command:{status:'exited',code:2}}},false,false),'Command failed (exit 2)');
 assert.equal(actionDuration({tool_outcome:{elapsed_ms:1234}}),'1.2 s');
 assert.equal(actionDuration({}),'timing unavailable');
});
