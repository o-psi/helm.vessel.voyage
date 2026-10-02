// Passive metrics for an already authorized, qualification-owned Chromium page.
// No navigation, login, effects, payload recording or clock manipulation.
import {performance} from 'node:perf_hooks';

const metricNames=['TaskDuration','ScriptDuration','LayoutDuration','RecalcStyleDuration',
 'JSHeapUsedSize','JSHeapTotalSize','Nodes','Documents','JSEventListeners'];
export async function browserCost(context,page,{webSocketUrls=[],nativeOperationUrls=[],maxMilliseconds=60000}={}){
 if(webSocketUrls.length>8||nativeOperationUrls.length>8||maxMilliseconds<1000||maxMilliseconds>60000)throw Error('measurement scope exceeds bound');
 const selected=new Set(webSocketUrls),httpSelected=new Set(nativeOperationUrls),sockets=new Set(),requests=new Set();
 const traffic={ws_sent_payload_bytes:0,ws_received_payload_bytes:0,ws_sent_frames:0,ws_received_frames:0,
  selected_ws_connections_seen:0,http_operation_request_payload_bytes:0,http_operation_requests:0,
  http_operation_response_payload_bytes:0,http_operation_response_encoded_bytes:0,http_operation_responses:0,
  http_operation_failures:0,http_operation_body_unavailable:0,http_operation_inflight_at_window_end:0};
 const cdp=await context.newCDPSession(page),handlers=[],bodyReads=new Set();let overflow=false;
 const on=(name,handler)=>{cdp.on(name,handler);handlers.push([name,handler]);};
 on('Network.webSocketCreated',event=>{if(selected.has(event.url)){if(sockets.size>=64){overflow=true;return;}sockets.add(event.requestId);traffic.selected_ws_connections_seen++;}});
 on('Network.webSocketClosed',event=>sockets.delete(event.requestId));
 for(const [name,direction] of [['Network.webSocketFrameSent','sent'],['Network.webSocketFrameReceived','received']]){
  on(name,event=>{
   if(!sockets.has(event.requestId))return;
   // Frame contents (including auth/private input) are transient only. Retain
   // byte totals, never request IDs, messages, URLs, cookies or headers.
   const frame=event.response,bytes=frame.opcode===2?Buffer.from(frame.payloadData,'base64').length:Buffer.byteLength(frame.payloadData,'utf8');
   traffic[`ws_${direction}_payload_bytes`]+=bytes;traffic[`ws_${direction}_frames`]++;
  });
 }
 on('Network.requestWillBeSent',event=>{if(httpSelected.has(event.request.url)){
  if(requests.size>=128||requests.has(event.requestId)||event.redirectResponse){overflow=true;return;}requests.add(event.requestId);traffic.http_operation_requests++;
  if(event.request.hasPostData&&typeof event.request.postData!=='string'){traffic.http_operation_body_unavailable++;return;}
  const bytes=Buffer.byteLength(event.request.postData||'', 'utf8');
  if(bytes>4*1024*1024){overflow=true;return;}traffic.http_operation_request_payload_bytes+=bytes;
 }});
 on('Network.loadingFailed',event=>{if(requests.delete(event.requestId))traffic.http_operation_failures++;});
 on('Network.loadingFinished',event=>{if(requests.delete(event.requestId)){
  if(!Number.isSafeInteger(event.encodedDataLength)||event.encodedDataLength<0){overflow=true;return;}
  traffic.http_operation_response_encoded_bytes+=event.encodedDataLength;traffic.http_operation_responses++;
  if(bodyReads.size>=128){overflow=true;return;}
  // Count the actual decoded application body, never retain/return its content.
  const reading=(async()=>{try{
   const body=await cdp.send('Network.getResponseBody',{requestId:event.requestId});
   if(typeof body.body!=='string'||body.body.length>8*1024*1024){overflow=true;return;}
   const bytes=body.base64Encoded?Buffer.from(body.body,'base64').length:Buffer.byteLength(body.body,'utf8');
   if(bytes>4*1024*1024){overflow=true;return;}traffic.http_operation_response_payload_bytes+=bytes;
  }catch{traffic.http_operation_body_unavailable++;}})();
  bodyReads.add(reading);void reading.finally(()=>bodyReads.delete(reading));
 }});
 try{await cdp.send('Performance.enable');await cdp.send('Network.enable');}
 catch(error){for(const [name,handler] of handlers)cdp.off(name,handler);await cdp.detach();throw error;}
 const read=async()=>{
  const result=await cdp.send('Performance.getMetrics'),all=Object.fromEntries(result.metrics.map(m=>[m.name,m.value]));
  return Object.fromEntries(metricNames.map(name=>[name,Number.isFinite(all[name])?all[name]:null]));
 };
 let before;
 try{before=await read();}catch(error){for(const [name,handler] of handlers)cdp.off(name,handler);await cdp.detach();throw error;}
 const started=performance.now();let finishing=null,timer;
 const finish=()=>finishing??=(async()=>{
   clearTimeout(timer);
   try{
    for(const [name,handler] of handlers)cdp.off(name,handler);
    traffic.http_operation_inflight_at_window_end=requests.size;
    await Promise.race([Promise.all([...bodyReads]),new Promise(resolve=>setTimeout(()=>{if(bodyReads.size)traffic.http_operation_body_unavailable+=bodyReads.size;resolve();},1000))]);
    return {elapsed_ms:performance.now()-started,before,after:await read(),traffic:{...traffic},
    status:overflow?'scope_overflow':traffic.http_operation_failures||traffic.http_operation_body_unavailable?'unknown':'observed',websocket_traffic_qualified:selected.size>0&&traffic.selected_ws_connections_seen>0,
    cpu_scope:'renderer task time; whole-browser OS CPU/RSS require the owned PID ledger',
    traffic_scope:'selected WebSocket payload and native HTTP requests begun/responses completed in observation window (inflight-at-end counted separately); UTF8 request/decoded response application bytes; encoded response transfer bytes separate; excludes headers/TCP/TLS and native Helm Vessel socket bytes'};}
   finally{for(const [name,handler] of handlers)cdp.off(name,handler);await cdp.detach();}
  })();
 timer=setTimeout(()=>{void finish().catch(()=>{});},maxMilliseconds);
 return {async snapshot(){if(finishing)throw Error('measurement finished; read stop result');return {elapsed_ms:performance.now()-started,renderer:await read(),traffic:{...traffic}};},stop:finish};
}

export async function inputVisibleLatency(action,visible){
 const started=performance.now();await action();await visible();
 return {input_to_visible_ms:performance.now()-started,scope:'single explicit input and observed replay marker; no retries'};
}
