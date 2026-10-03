// Synthetic private CDP event-source contracts; no native/browser/TLS claim.
import test from 'node:test';import assert from 'node:assert/strict';import {EventEmitter} from 'node:events';
import {browserCost} from './host_browser_client_cost.mjs';
const url='http://127.0.0.1:1/operation';
function fixture(){
 const cdp=new EventEmitter();let reads=0;const calls=[];
 cdp.send=async(name)=>{calls.push(name);if(name==='Performance.getMetrics')return{metrics:[{name:'TaskDuration',value:++reads},{name:'JSHeapUsedSize',value:123},{name:'Nodes',value:12}]};return{};};
 cdp.detach=async()=>{cdp.detached=true;};return{cdp,calls,context:{newCDPSession:async()=>cdp}};
}
const start=(f,id='owned',body='private 日本語')=>f.cdp.emit('Network.requestWillBeSent',{requestId:id,request:{url,hasPostData:true,postData:body}});
const chunk=(f,id='owned',decoded=19,encoded=7)=>f.cdp.emit('Network.dataReceived',{requestId:id,timestamp:1,dataLength:decoded,encodedDataLength:encoded});
const finish=(f,id='owned')=>f.cdp.emit('Network.loadingFinished',{requestId:id,encodedDataLength:999});
const meter=f=>browserCost(f.context,{}, {nativeOperationUrls:[url],maxMilliseconds:1000});
test('selected chunk byte totals exclude ambient content and completion transfer totals',async()=>{
 const f=fixture(),m=await meter(f);start(f);chunk(f);chunk(f,'owned',13,5);
 f.cdp.emit('Network.responseReceived',{requestId:'ambient',response:{url:'https://unrelated.invalid/'}});chunk(f,'ambient',999,999);finish(f,'ambient');finish(f);
 const v=await m.stop();assert.equal(v.status,'observed');assert.equal(v.traffic.http_operation_request_payload_bytes,Buffer.byteLength('private 日本語'));assert.equal(v.traffic.http_operation_response_payload_bytes,32);assert.equal(v.traffic.http_operation_response_encoded_bytes,12);assert.equal(v.traffic.http_operation_responses,1);assert.equal(v.traffic.http_operation_data_chunks,2);assert.ok(!JSON.stringify(v).includes('private'));assert.ok(!f.calls.includes('Network.getResponseBody'));assert.equal(f.cdp.detached,true);assert.equal(f.cdp.eventNames().length,0);
});
test('request spanning end contributes only observed chunks and explicitly remains inflight',async()=>{
 const f=fixture(),m=await meter(f);start(f);chunk(f,'owned',11,8);const v=await m.stop();chunk(f,'owned',100,100);finish(f);
 assert.equal(v.status,'observed');assert.equal(v.traffic.http_operation_inflight_at_window_end,1);assert.equal(v.traffic.http_operation_responses,0);assert.equal(v.traffic.http_operation_response_payload_bytes,11);assert.equal(v.traffic.http_operation_response_encoded_bytes,8);assert.deepEqual(v.qualification_gaps,[]);
});
test('request begun before window counts only its in-window response chunks',async()=>{
 const f=fixture(),original=f.cdp.send;f.cdp.send=async name=>{if(name==='Network.enable'){start(f,'early','outside-window');chunk(f,'early',900,800);}return original(name);};
 const m=await meter(f);chunk(f,'early',17,9);finish(f,'early');const v=await m.stop();assert.equal(v.status,'observed');assert.equal(v.prewindow_requests_crossing,1);assert.equal(v.traffic.http_operation_requests,0);assert.equal(v.traffic.http_operation_request_payload_bytes,0);assert.equal(v.traffic.http_operation_response_payload_bytes,17);assert.equal(v.traffic.http_operation_response_completions_from_before_window,1);
});
test('response URL scopes a request already active before CDP subscription',async()=>{
 const f=fixture(),m=await meter(f);f.cdp.emit('Network.responseReceived',{requestId:'preexisting',response:{url}});chunk(f,'preexisting',23,0);const v=await m.stop();
 assert.equal(v.status,'observed');assert.equal(v.traffic.http_operation_requests,0);assert.equal(v.traffic.http_operation_response_payload_bytes,23);assert.equal(v.traffic.http_operation_response_encoded_bytes,0);assert.equal(v.traffic.http_operation_preexisting_inflight_at_window_end,1);
});
test('completed empty response is zero bytes without a guessed full-body read',async()=>{
 const f=fixture(),m=await meter(f);start(f,'owned','{}');finish(f);const v=await m.stop();assert.equal(v.status,'observed');assert.equal(v.traffic.http_operation_response_payload_bytes,0);assert.equal(v.traffic.http_operation_response_encoded_bytes,0);assert.equal(v.traffic.http_operation_responses,1);
});
test('missing request payload and known transport failures remain unknown',async()=>{
 for(const kind of ['missing','failed','preexisting-failed']){const f=fixture(),m=await meter(f);
 if(kind==='preexisting-failed')f.cdp.emit('Network.responseReceived',{requestId:'owned',response:{url}});
 else f.cdp.emit('Network.requestWillBeSent',{requestId:'owned',request:{url,hasPostData:true,...(kind==='missing'?{}:{postData:'{}'})}});
 if(kind!=='missing')f.cdp.emit('Network.loadingFailed',{requestId:'owned'});
 const v=await m.stop();assert.equal(v.status,'unknown');assert.ok(v.qualification_gaps.includes(kind==='missing'?'http_body_unavailable':'http_operation_failed'));}
});
test('invalid or missing chunk counts refuse byte attribution',async()=>{
 for(const fields of [{dataLength:-1},{encodedDataLength:null},{dataLength:1.5},{dataLength:4*1024*1024+1},{timestamp:null},{timestamp:NaN}]){const f=fixture(),m=await meter(f);start(f);f.cdp.emit('Network.dataReceived',{requestId:'owned',timestamp:1,dataLength:4,encodedDataLength:3,...fields});finish(f);const v=await m.stop();assert.equal(v.status,'unknown');assert.equal(v.traffic.http_operation_body_unavailable,1);assert.equal(v.traffic.http_operation_response_payload_bytes,0);}
});
test('empty or silent selected sources cannot pass',async()=>{
 const f=fixture(),m=await meter(f);assert.equal((await m.stop()).status,'unknown');const w=fixture(),ws=await browserCost(w.context,{}, {webSocketUrls:['wss://fixture.invalid/socket'],maxMilliseconds:1000});assert.equal((await ws.stop()).status,'unknown');
});
test('missing essential renderer metrics cannot pass',async()=>{
 for(const missing of ['TaskDuration','JSHeapUsedSize','Nodes']){const f=fixture(),original=f.cdp.send;f.cdp.send=async name=>{const value=await original(name);if(name==='Performance.getMetrics')value.metrics=value.metrics.filter(x=>x.name!==missing);return value;};const m=await meter(f);start(f);chunk(f);finish(f);const v=await m.stop();assert.equal(v.status,'unknown');assert.equal(v.renderer_metrics_qualified,false);}
});
test('selected HTTP request scope overflow remains refusing',async()=>{
 const f=fixture(),m=await meter(f);for(let i=0;i<129;i++)start(f,String(i),'{}');const v=await m.stop();assert.equal(v.status,'scope_overflow');assert.ok(v.qualification_gaps.includes('scope_overflow'));
});
