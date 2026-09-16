import test from 'node:test';
import assert from 'node:assert/strict';
import {connectionDiagnostic} from '../resources/js/connection-diagnostics.js';
test('connection diagnostics never include supplied content, secrets or error objects',()=>{
 const original=console.debug;let captured;
 console.debug=(...args)=>{captured=args;};
 try { connectionDiagnostic('request',{op:'submit',request_id:'fixture',prompt:'secret text',ticket:'credential',url:'private',error:new Error('sensitive')}); }
 finally {console.debug=original;}
 assert.equal(captured[1].op,'submit');
 assert.doesNotMatch(JSON.stringify(captured),/secret text|credential|private|sensitive/);
});
