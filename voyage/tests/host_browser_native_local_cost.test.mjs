// Pure private event-source contracts, no Chromium/native/TLS/process claim.
import test from 'node:test';import assert from 'node:assert/strict';import {EventEmitter} from 'node:events';
import {browserCost} from './host_browser_client_cost.mjs';
function fixture(body='private fixture 日本語'){
 const cdp=new EventEmitter();let reads=0;cdp.detached=false;
 cdp.send=async(name)=>{if(name==='Performance.getMetrics')return{metrics:[{name:'TaskDuration',value:++reads},{name:'JSHeapUsedSize',value:123},{name:'Nodes',value:12}]};if(name==='Network.getResponseBody')return{body,base64Encoded:false};return{};};
 cdp.detach=async()=>{cdp.detached=true;};return{cdp,context:{newCDPSession:async()=>cdp}};
}
test('selected native requests and decoded response bodies counted without content or ambient traffic',async()=>{
 const f=fixture();const meter=await browserCost(f.context,{}, {nativeOperationUrls:['http://127.0.0.1:1234/operation'],maxMilliseconds:1000});
 f.cdp.emit('Network.requestWillBeSent',{requestId:'owned',request:{url:'http://127.0.0.1:1234/operation',hasPostData:true,postData:'private synthetic request 日本語'}});
 f.cdp.emit('Network.requestWillBeSent',{requestId:'ambient',request:{url:'https://unrelated.invalid/',hasPostData:true,postData:'unrelated sentinel'}});
 f.cdp.emit('Network.loadingFinished',{requestId:'owned',encodedDataLength:456});
 f.cdp.emit('Network.loadingFinished',{requestId:'ambient',encodedDataLength:999});
 const value=await meter.stop();assert.equal(value.status,'observed');
 assert.equal(value.traffic.http_operation_request_payload_bytes,Buffer.byteLength('private synthetic request 日本語'));
 assert.equal(value.traffic.http_operation_response_payload_bytes,Buffer.byteLength('private fixture 日本語'));
 assert.equal(value.traffic.http_operation_response_encoded_bytes,456);assert.equal(value.traffic.http_operation_requests,1);
 assert.ok(!JSON.stringify(value).includes('private fixture'));assert.ok(!JSON.stringify(value).includes('unrelated sentinel'));
 assert.equal(f.cdp.detached,true);assert.equal(f.cdp.eventNames().length,0);
});
test('missing private request body or failed transport cannot be passing cost evidence',async()=>{
 for(const failure of ['missing','failed']){const f=fixture();const meter=await browserCost(f.context,{}, {nativeOperationUrls:['http://127.0.0.1:1/operation'],maxMilliseconds:1000});
 f.cdp.emit('Network.requestWillBeSent',{requestId:'owned',request:{url:'http://127.0.0.1:1/operation',hasPostData:failure==='missing'}});
 f.cdp.emit('Network.loadingFailed',{requestId:'owned'});const value=await meter.stop();assert.equal(value.status,'unknown');assert.equal(value.traffic.http_operation_failures,1);}
});
test('crossing window request stays explicitly inflight not a fabricated response byte count',async()=>{
 const f=fixture();const meter=await browserCost(f.context,{}, {nativeOperationUrls:['http://127.0.0.1:1/operation'],maxMilliseconds:1000});
 f.cdp.emit('Network.requestWillBeSent',{requestId:'owned',request:{url:'http://127.0.0.1:1/operation',postData:'{}'}});
 const value=await meter.stop();assert.equal(value.status,'unknown');assert.equal(value.traffic.http_operation_inflight_at_window_end,1);assert.equal(value.traffic.http_operation_response_payload_bytes,0);
 assert.ok(value.traffic_scope.includes('observation window'));
});
test('unavailable response body refuses complete byte attribution',async()=>{
 const f=fixture();const original=f.cdp.send;f.cdp.send=async(name)=>{if(name==='Network.getResponseBody')throw Error('private unavailable');return original(name);};
 const meter=await browserCost(f.context,{}, {nativeOperationUrls:['http://127.0.0.1:1/operation'],maxMilliseconds:1000});
 f.cdp.emit('Network.requestWillBeSent',{requestId:'owned',request:{url:'http://127.0.0.1:1/operation',postData:'{}'}});
 f.cdp.emit('Network.loadingFinished',{requestId:'owned',encodedDataLength:7});const value=await meter.stop();assert.equal(value.status,'unknown');assert.equal(value.traffic.http_operation_body_unavailable,1);
});

test('empty selected native source and missing/nonboolean encoding stay unknown',async()=>{
 const empty=fixture();const zero=await browserCost(empty.context,{}, {nativeOperationUrls:['http://127.0.0.1:1/operation'],maxMilliseconds:1000});
 assert.equal((await zero.stop()).status,'unknown');
 for(const encoding of [undefined,null,'false',0]){
  const f=fixture();const original=f.cdp.send;f.cdp.send=async(name)=>name==='Network.getResponseBody'?{body:'private encoding fixture',base64Encoded:encoding}:original(name);
  const meter=await browserCost(f.context,{}, {nativeOperationUrls:['http://127.0.0.1:1/operation'],maxMilliseconds:1000});
  f.cdp.emit('Network.requestWillBeSent',{requestId:'owned',request:{url:'http://127.0.0.1:1/operation',postData:'{}'}});
  f.cdp.emit('Network.loadingFinished',{requestId:'owned',encodedDataLength:42});
  const value=await meter.stop();assert.equal(value.status,'unknown');assert.equal(value.traffic.http_operation_body_unavailable,1);
  assert.equal(value.traffic.http_operation_response_payload_bytes,0);assert.ok(!JSON.stringify(value).includes('private encoding fixture'));
 }
});

test('missing essential actual renderer task or heap measurement cannot pass',async()=>{
 for(const missing of ['TaskDuration','JSHeapUsedSize','Nodes']){
  const f=fixture();const original=f.cdp.send;f.cdp.send=async(name)=>{const result=await original(name);if(name==='Performance.getMetrics')result.metrics=result.metrics.filter(metric=>metric.name!==missing);return result;};
  const meter=await browserCost(f.context,{}, {maxMilliseconds:1000});const value=await meter.stop();
  assert.equal(value.status,'unknown');assert.equal(value.renderer_metrics_qualified,false);assert.equal(value.after[missing],null);
 }
});

test('prewindow request crossing and selected silent websocket source are unknown',async()=>{
 const f=fixture();const original=f.cdp.send;f.cdp.send=async(name)=>{if(name==='Network.enable')f.cdp.emit('Network.requestWillBeSent',{requestId:'early',request:{url:'http://127.0.0.1:1/operation',postData:'prewindow fixture'}});return original(name);};
 const meter=await browserCost(f.context,{}, {nativeOperationUrls:['http://127.0.0.1:1/operation'],maxMilliseconds:1000});
 f.cdp.emit('Network.loadingFinished',{requestId:'early',encodedDataLength:5});const value=await meter.stop();
 assert.equal(value.status,'unknown');assert.ok(value.prewindow_requests_crossing>0);assert.equal(value.traffic.http_operation_request_payload_bytes,0);assert.equal(value.traffic.http_operation_response_payload_bytes,0);
 const w=fixture();const silent=await browserCost(w.context,{}, {webSocketUrls:['wss://fixture.invalid/socket'],maxMilliseconds:1000});
 const missing=await silent.stop();assert.equal(missing.status,'unknown');assert.equal(missing.websocket_traffic_qualified,false);
});
