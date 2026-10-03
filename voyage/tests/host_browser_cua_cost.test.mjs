import test from 'node:test';
import assert from 'node:assert/strict';
import {cuaBrowserCost} from './host_browser_cua_cost.mjs';
const socketUrl='wss://fixture.example/v1/vessel/browser-socket';
const options={label:'a',socketUrl,tabId:'3',connectionId:'own-connection',vesselId:'owned-vessel',sessionIds:['session-a','session-b']};
const metricNames=['TaskDuration','ScriptDuration','LayoutDuration','RecalcStyleDuration','JSHeapUsedSize','JSHeapTotalSize','Nodes','Documents','JSEventListeners'];
function fixture(){
 const calls=[],queue=[];let cursor=0,samples=0;
 const cap={async send(method,params,opts){
  calls.push({method,params,opts});
  assert.notEqual(method,'Target.getTargetInfo');
  assert.equal(opts,undefined);
  if(method==='Performance.getMetrics'){samples++;return {metrics:metricNames.map((name,i)=>({name,value:100+i+samples}))};}
  return {};
 },async readEvents(opts){calls.push({method:'readEvents',opts});if(opts.afterSequence===undefined)return {cursor,events:[],hasMore:false,truncated:false};
  return queue.shift()||{cursor,events:[],hasMore:false,truncated:false};}};
 const tab={id:'3',capabilities:{async get(name){calls.push({method:'getCapability',name});assert.equal(name,'cdp');return cap;}}};
 const add=(events,extra={})=>{cursor+=events.length;queue.push({cursor,events:events.map((event,i)=>({sequence:cursor-events.length+i+1,source:{tabId:3},...event})),hasMore:false,truncated:false,...extra});};
 return {tab,calls,add};
}
const frame=(method,payload,id='own-socket')=>({method,params:{requestId:id,response:{opcode:1,payloadData:payload}}});
const ack={method:'Runtime.consoleAPICalled',params:{args:[{value:'[Helm connection]'},{preview:{properties:[{name:'event',value:'connected'},{name:'connection',value:'own-connection'}]}}]}};
const auth=JSON.stringify({type:'authenticate',token:'a'.repeat(64)});
const hello=id=>JSON.stringify({type:'hello',protocol:1,vessel_id:options.vesselId,socket_id:'server-'+id});
function opening(id='own-socket',renewing=false){return [
 {method:'Network.webSocketCreated',params:{requestId:id,url:socketUrl}},
 frame('Network.webSocketFrameSent',auth,id),frame('Network.webSocketFrameReceived',hello(id),id),
 {method:'Runtime.consoleAPICalled',params:{args:[{value:'[Helm connection]'},{preview:{properties:[{name:'event',value:renewing?'renewal_ack':'connected'},{name:'connection',value:options.connectionId}]}}]}}
];}
test('real documented event boundary reduces payloads to counts and renderer deltas',async()=>{
 const f=fixture(),observer=await cuaBrowserCost(f.tab,options);
 f.add(opening());
 await observer.begin();
 const data='{"type":"command","request_id":"r","request":{"command":{"op":"host_browser","session_id":"session-a","operation":{"action":"input","command_id":"effect-a","input":{"text":"SYNTHETIC_PRIVATE_INPUT"}}}}}';
 const replied='{"type":"reply","request_id":"r","response":{"outcome_unknown":false,"error":null,"result":{"result":{}}}}';
 f.add([frame('Network.webSocketFrameSent',data),frame('Network.webSocketFrameReceived','日本語'),frame('Network.webSocketFrameReceived',replied),
  ...opening('replacement-socket',true),
  {method:'Network.webSocketClosed',params:{requestId:'own-socket'}}]);
 const result=await observer.finish();
 assert.equal(result.sent_bytes,new TextEncoder().encode(data+auth).length);
 assert.equal(result.received_bytes,9+new TextEncoder().encode(replied+hello('replacement-socket')).length);assert.equal(result.task_seconds,1);assert.equal(result.node_delta,1);assert.equal(result.renewal_acks,1);assert.equal(result.status,'observed');
 const output=JSON.stringify(result);for(const value of [socketUrl,'SYNTHETIC_PRIVATE_TOKEN','SYNTHETIC_PRIVATE_INPUT','own-target','own-socket'])assert.equal(output.includes(value),false);
 assert.ok(f.calls.filter(c=>c.method==='readEvents').every(c=>!Object.hasOwn(c.opts,'target')));
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
 f.add(opening(),{hasMore:true});
 f.add([]);await observer.begin();
 const data='{"type":"command","request_id":"r","request":{"command":{"op":"host_browser","session_id":"session-a","operation":{"action":"close","command_id":"close-a"}}}}';
 f.add([frame('Network.webSocketFrameSent',data),frame('Network.webSocketFrameSent',data)]);
 const result=await observer.finish();assert.equal(result.duplicate_effects,1);
 const reads=f.calls.filter(c=>c.method==='readEvents'&&c.opts.afterSequence!==undefined);
 assert.ok(reads.every((c,i)=>i===0||c.opts.afterSequence>=reads[i-1].opts.afterSequence));await observer.stop();
});
test('clean close or unacknowledged replacement cannot reuse old authenticated health',async()=>{
 const f=fixture(),observer=await cuaBrowserCost(f.tab,options);
 f.add(opening());await observer.begin();
 f.add([{method:'Network.webSocketClosed',params:{requestId:'own-socket'}},
  {method:'Network.webSocketCreated',params:{requestId:'unacknowledged',url:socketUrl}}]);
 await assert.rejects(observer.finish,/observation unavailable/);await observer.stop();
});

test('selected-tab identity is exact before acquiring a capability',async()=>{
 for(const tabId of ['4','03',3.5,Number.MAX_SAFE_INTEGER+1,null]){
  const f=fixture();await assert.rejects(cuaBrowserCost(f.tab,{...options,tabId}),/observation unavailable/);
  assert.equal(f.calls.length,0);
 }
 const f=fixture(),observer=await cuaBrowserCost(f.tab,{...options,tabId:3});await observer.stop();
});
test('events require selected-tab identity and consistent optional target metadata',async()=>{
 for(const source of [{},{tabId:'03'},{tabId:4},{tabId:3,targetId:''},{targetId:'own-target'}]){
  const f=fixture(),observer=await cuaBrowserCost(f.tab,options);
  f.add([{method:'Network.webSocketCreated',params:{requestId:'own-socket',url:socketUrl},source}]);
  await assert.rejects(observer.poll,/observation unavailable/);await observer.stop();
 }
 const f=fixture(),observer=await cuaBrowserCost(f.tab,options);
 f.add(opening().map(event=>({...event,source:{tabId:3,targetId:'own-target'}})));await observer.poll();
 f.add([{...ack,source:{tabId:'3',targetId:'different-target'}}]);
 await assert.rejects(observer.poll,/observation unavailable/);await observer.stop();
});

test('authenticated replacement overlaps draining predecessor without lost reply or replay',async()=>{
 const f=fixture(),observer=await cuaBrowserCost(f.tab,options);f.add(opening());await observer.begin();
 const command=JSON.stringify({type:'command',request_id:'pending',request:{command:{op:'host_browser',session_id:'session-a',operation:{action:'close',command_id:'once'}}}});
 f.add([frame('Network.webSocketFrameSent',command),...opening('replacement',true),frame('Network.webSocketFrameReceived',JSON.stringify({type:'reply',request_id:'pending',response:{outcome_unknown:false,error:null,result:{result:{}}}}))]);
 const result=await observer.finish();assert.equal(result.status,'observed');assert.equal(result.transport_at_end.authenticated_open_sockets,2);assert.equal(result.transport_at_end.draining_authenticated_sockets,1);assert.equal(result.renewal_acks,1);assert.equal(result.pending_effects,0);assert.equal(result.duplicate_effects,0);
 f.add([{method:'Network.webSocketClosed',params:{requestId:'own-socket'}}]);await observer.begin();assert.equal((await observer.finish()).transport_at_end.authenticated_open_sockets,1);await observer.stop();
});
test('opening replacement at boundary is explicit and cannot borrow authentication after losing active socket',async()=>{
 const f=fixture(),observer=await cuaBrowserCost(f.tab,options);f.add(opening());await observer.begin();f.add(opening('replacement',true).slice(0,2));
 const result=await observer.finish();assert.equal(result.status,'observed');assert.equal(result.transport_at_end.pending_handshakes,1);assert.equal(result.transport_at_end.authenticated_open_sockets,1);
 f.add([{method:'Network.webSocketClosed',params:{requestId:'own-socket'}}]);await assert.rejects(observer.poll);await assert.rejects(observer.begin);await observer.stop();
});
test('wrong Vessel hello, duplicate acknowledgement, socket error and lost effect reply remain terminal',async()=>{
 for(const kind of ['wrong-vessel','duplicate-ack','socket-error','lost-effect']){
  const f=fixture(),observer=await cuaBrowserCost(f.tab,options);f.add(opening());await observer.begin();
  if(kind==='wrong-vessel'){const events=opening('replacement',true);events[2]=frame('Network.webSocketFrameReceived',JSON.stringify({type:'hello',protocol:1,vessel_id:'other',socket_id:'server-new'}),'replacement');f.add(events);}
  else if(kind==='duplicate-ack')f.add([ack]);
  else if(kind==='socket-error')f.add([{method:'Network.webSocketFrameError',params:{requestId:'own-socket',errorMessage:'not retained'}}]);
  else f.add([frame('Network.webSocketFrameSent',JSON.stringify({type:'command',request_id:'r',request:{command:{op:'host_browser',session_id:'session-a',operation:{action:'close',command_id:'once'}}}})),...opening('replacement',true),{method:'Network.webSocketClosed',params:{requestId:'own-socket'}}]);
  await assert.rejects(observer.finish);await assert.rejects(observer.poll);await observer.stop();
 }
});
test('unauthenticated hello or an unexplained third socket cannot qualify',async()=>{
 for(const kind of ['no-auth','third']){const f=fixture(),observer=await cuaBrowserCost(f.tab,options);f.add(opening());await observer.begin();
  if(kind==='no-auth')f.add([opening('replacement',true)[0],opening('replacement',true)[2]]);
  else f.add([...opening('replacement',true),opening('third',true)[0]]);
  await assert.rejects(observer.finish);await observer.stop();}
});

test('pending handshake and draining predecessor retain Fleet deadlines without recovery from refusal',async()=>{
 for(const kind of ['handshake','drain']){
  const f=fixture(),observer=await cuaBrowserCost(f.tab,options);f.add(opening());await observer.begin();f.add(opening('replacement',true).slice(0,kind==='handshake'?2:4));await observer.poll();
  const now=Date.now;const advanced=now()+(kind==='handshake'?10001:30001);Date.now=()=>advanced;
  try{await assert.rejects(observer.finish);}finally{Date.now=now;}
  await assert.rejects(observer.poll);await observer.stop();
 }
});
test('duplicate effects across replacement sockets remain unknown',async()=>{
 const f=fixture(),observer=await cuaBrowserCost(f.tab,options);f.add(opening());await observer.begin();
 const command=id=>JSON.stringify({type:'command',request_id:id,request:{command:{op:'host_browser',session_id:'session-a',operation:{action:'close',command_id:'same-effect'}}}});
 f.add([frame('Network.webSocketFrameSent',command('old')),frame('Network.webSocketFrameReceived',JSON.stringify({type:'reply',request_id:'old',response:{outcome_unknown:false,error:null,result:{result:{}}}})),...opening('replacement',true),frame('Network.webSocketFrameSent',command('new'),'replacement')]);
 const value=await observer.finish();assert.equal(value.status,'unknown');assert.equal(value.duplicate_effects,1);await observer.stop();
});

test('lost selected-tab capability cannot leave the prior authenticated socket passing',async()=>{
 const f=fixture(),observer=await cuaBrowserCost(f.tab,options);f.add(opening());await observer.begin();const cap=await f.tab.capabilities.get('cdp');cap.readEvents=async()=>{throw Error('selected tab unavailable');};await assert.rejects(observer.finish);await assert.rejects(observer.poll);await observer.stop();
});
