import test from 'node:test';
import assert from 'node:assert/strict';
import {cuaBrowserCost} from './host_browser_cua_cost.mjs';
const socketUrl='wss://fixture.example/v1/vessel/browser-socket';
const options={label:'a',socketUrl,tabId:'own-tab',connectionId:'own-connection',sessionIds:['session-a','session-b']};
const metricNames=['TaskDuration','ScriptDuration','LayoutDuration','RecalcStyleDuration','JSHeapUsedSize','JSHeapTotalSize','Nodes','Documents','JSEventListeners'];
function fixture(){
 const calls=[],queue=[];let cursor=0,samples=0;
 const cap={async send(method,params,opts){
  calls.push({method,params,opts});
  if(method==='Target.getTargetInfo')return {targetInfo:{targetId:'own-target',url:'https://private-fixture.example/never-return'}};
  if(method==='Performance.getMetrics'){samples++;return {metrics:metricNames.map((name,i)=>({name,value:100+i+samples}))};}
  return {};
 },async readEvents(opts){calls.push({method:'readEvents',opts});if(opts.afterSequence===undefined)return {cursor,events:[],hasMore:false,truncated:false};
  return queue.shift()||{cursor,events:[],hasMore:false,truncated:false};}};
 const tab={capabilities:{async get(name){assert.equal(name,'cdp');return cap;}}};
 const add=(events,extra={})=>{cursor+=events.length;queue.push({cursor,events:events.map((event,i)=>({sequence:cursor-events.length+i+1,source:{tabId:'own-tab',targetId:'own-target'},...event})),hasMore:false,truncated:false,...extra});};
 return {tab,calls,add};
}
const frame=(method,payload)=>({method,params:{requestId:'own-socket',response:{opcode:1,payloadData:payload}}});
const ack={method:'Runtime.consoleAPICalled',params:{args:[{value:'[Helm connection]'},{preview:{properties:[{name:'event',value:'connected'},{name:'connection',value:'own-connection'}]}}]}};
test('real documented event boundary reduces payloads to counts and renderer deltas',async()=>{
 const f=fixture(),observer=await cuaBrowserCost(f.tab,options);
 f.add([{method:'Network.webSocketCreated',params:{requestId:'own-socket',url:socketUrl}},
  frame('Network.webSocketFrameSent','{"type":"authenticate","token":"SYNTHETIC_PRIVATE_TOKEN"}'),ack]);
 await observer.begin();
 const data='{"type":"command","request_id":"r","request":{"command":{"op":"host_browser","session_id":"session-a","operation":{"action":"input","command_id":"effect-a","input":{"text":"SYNTHETIC_PRIVATE_INPUT"}}}}}';
 const replied='{"type":"reply","request_id":"r","response":{"outcome_unknown":false,"error":null,"result":{"result":{}}}}';
 f.add([frame('Network.webSocketFrameSent',data),frame('Network.webSocketFrameReceived','日本語'),frame('Network.webSocketFrameReceived',replied),
  {method:'Network.webSocketCreated',params:{requestId:'replacement-socket',url:socketUrl}},
  {method:'Network.webSocketClosed',params:{requestId:'own-socket'}},
  {method:'Runtime.consoleAPICalled',params:{args:[{value:'[Helm connection]'},{preview:{properties:[{name:'event',value:'renewal_ack'},{name:'connection',value:'own-connection'}]}}]}}]);
 const result=await observer.finish();
 assert.equal(result.sent_bytes,new TextEncoder().encode(data).length);
 assert.equal(result.received_bytes,9+new TextEncoder().encode(replied).length);assert.equal(result.task_seconds,1);assert.equal(result.node_delta,1);assert.equal(result.renewal_acks,1);assert.equal(result.status,'observed');
 const output=JSON.stringify(result);for(const value of [socketUrl,'SYNTHETIC_PRIVATE_TOKEN','SYNTHETIC_PRIVATE_INPUT','own-target','own-socket'])assert.equal(output.includes(value),false);
 assert.ok(f.calls.filter(c=>c.method==='readEvents').every(c=>c.opts.target.targetId==='own-target'));
 await observer.stop();
});
test('truncated event retention and wrong-tab attribution refuse without invented zeros',async()=>{
 for(const mode of ['truncated','wrong-tab']){
  const f=fixture(),observer=await cuaBrowserCost(f.tab,options);
  f.add([{method:'Network.webSocketCreated',params:{requestId:'own-socket',url:socketUrl},
   ...(mode==='wrong-tab'?{source:{tabId:'different-tab',targetId:'own-target'}}:{})}],{truncated:mode==='truncated'});
  await assert.rejects(observer.poll,/observation unavailable/);await observer.stop();
 }
});
test('an already-open socket without a created event cannot become measured traffic',async()=>{
 const f=fixture(),observer=await cuaBrowserCost(f.tab,options);
 f.add([frame('Network.webSocketFrameReceived','not attributable to a discovered socket')]);
 await assert.rejects(observer.begin,/observation unavailable/);await observer.stop();
});
test('event pagination retains cursor and metadata flags effect replay',async()=>{
 const f=fixture(),observer=await cuaBrowserCost(f.tab,options);
 f.add([{method:'Network.webSocketCreated',params:{requestId:'own-socket',url:socketUrl}},ack],{hasMore:true});
 f.add([]);await observer.begin();
 const data='{"type":"command","request_id":"r","request":{"command":{"op":"host_browser","session_id":"session-a","operation":{"action":"close","command_id":"close-a"}}}}';
 f.add([frame('Network.webSocketFrameSent',data),frame('Network.webSocketFrameSent',data)]);
 const result=await observer.finish();assert.equal(result.duplicate_effects,1);
 const reads=f.calls.filter(c=>c.method==='readEvents'&&c.opts.afterSequence!==undefined);
 assert.ok(reads.every((c,i)=>i===0||c.opts.afterSequence>=reads[i-1].opts.afterSequence));await observer.stop();
});
test('clean close or unacknowledged replacement cannot reuse old authenticated health',async()=>{
 const f=fixture(),observer=await cuaBrowserCost(f.tab,options);
 f.add([{method:'Network.webSocketCreated',params:{requestId:'own-socket',url:socketUrl}},ack]);await observer.begin();
 f.add([{method:'Network.webSocketClosed',params:{requestId:'own-socket'}},
  {method:'Network.webSocketCreated',params:{requestId:'unacknowledged',url:socketUrl}}]);
 await assert.rejects(observer.finish,/observation unavailable/);await observer.stop();
});
