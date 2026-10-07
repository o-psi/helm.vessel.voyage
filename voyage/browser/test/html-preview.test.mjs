import test from 'node:test';
import assert from 'node:assert/strict';
import {validatePreview,htmlPreview} from '../html-preview.mjs';

test('preview input is bounded in UTF-8 and cannot choose arbitrary settings',()=>{
  validatePreview({html:'<p>x</p>',width:390,appearance:'light'});
  for(const a of [{html:'é'.repeat(131072),width:390,appearance:'light'},{html:' ',width:390,appearance:'dark'},{html:'x',width:2000,appearance:'dark'},{html:'x',width:390,appearance:'private'}])assert.throws(()=>validatePreview(a));
});
test('preview refuses failed render and observes disposable context cleanup',async()=>{
  let closed=0,routed=0;const context={setDefaultTimeout(){},setDefaultNavigationTimeout(){},route:async()=>routed++,routeWebSocket:async()=>routed++,addInitScript:async()=>{},newPage:async()=>{throw Error('render failed');},close:async()=>closed++};
  const worker={task:{newContext:async options=>{assert.equal(options.acceptDownloads,false);assert.equal(options.serviceWorkers,'block');return context;}}};
  await assert.rejects(htmlPreview(worker,{html:'<p>x</p>',width:390,appearance:'dark'}),/render failed/);
  assert.equal(routed,2);assert.equal(closed,1);assert.equal(worker.previewContext,null);
});
test('failed cleanup remains tracked rather than claiming observed closure',async()=>{
  const context={setDefaultTimeout(){},setDefaultNavigationTimeout(){},route:async()=>{},routeWebSocket:async()=>{},addInitScript:async()=>{},newPage:async()=>{throw Error('render failed');},close:async()=>{throw Error('cleanup failed');}};
  const worker={task:{newContext:async()=>context}};
  await assert.rejects(htmlPreview(worker,{html:'<p>x</p>',width:390,appearance:'dark'}),/cleanup failed/);assert.equal(worker.previewContext,context);
});

test('worker previews scripts in an opaque context without touching the task page',{timeout:20000},async t=>{
  const {Worker}=await import('../worker.mjs');
  const {randomUUID}=await import('node:crypto');
  const fs=await import('node:fs/promises'),os=await import('node:os'),path=await import('node:path');
  const root=await fs.mkdtemp(path.join(os.tmpdir(),'html-preview-worker-'));
  const worker=new Worker();let previewError;const previewDiagnostics=[];const perform=worker.perform.bind(worker);worker.perform=async(...args)=>{try{return await perform(...args);}catch(error){previewError=error;throw error;}};t.after(async()=>{await worker.dispose();await fs.rm(root,{recursive:true,force:true});});
  const call=async(op,args={})=>{const reply=await worker.request({id:randomUUID(),op,browser:worker.browser,epochs:{...worker.epochs},...args});assert.equal(reply.ok,true,String(previewError?.stack||JSON.stringify(reply))+JSON.stringify(previewDiagnostics));return reply.result;};
  await call('init',{config:{root,executable:'/usr/bin/chromium',public_web:false,origins:[],width:640,height:480}});
  await call('open');
  const newContext=worker.task.newContext.bind(worker.task);worker.task.newContext=async(...args)=>{const context=await newContext(...args);context.on('page',page=>{page.on('framenavigated',frame=>previewDiagnostics.push(frame.url()));page.on('console',m=>previewDiagnostics.push(m.text()));});return context;};
  const active=worker.active,tabCount=worker.tabs.size,url=worker.page.url();
  const html='<style>body{height:420px}</style><button onclick="this.textContent=\'Clicked\'">Mock</button><script>console.log("SCRIPT_WORKED");try{parent.document.body;console.error("PARENT_EXPOSED")}catch{console.log("PARENT_BLOCKED")}try{localStorage.getItem("private");console.error("STORAGE_EXPOSED")}catch{console.log("STORAGE_BLOCKED")}fetch("http://127.0.0.1:1/probe").catch(()=>console.log("NETWORK_BLOCKED"));</script>';
  const result=await call('agent',{action:{kind:'html_preview',html,width:390,appearance:'dark'}});
  const value=result.value;
  assert.equal(value.width,390);assert.equal(value.contentHeight,420);assert.equal(value.capturedHeight,420);
  assert.deepEqual(Buffer.from(value.data_base64,'base64').subarray(0,8),Buffer.from([137,80,78,71,13,10,26,10]));
  const diagnostics=value.consoleMessages.map(m=>m.text).join('\n');
  assert.match(diagnostics,/SCRIPT_WORKED/);assert.match(diagnostics,/PARENT_BLOCKED/);assert.match(diagnostics,/STORAGE_BLOCKED/);assert.match(diagnostics,/NETWORK_BLOCKED/);assert.doesNotMatch(diagnostics,/PARENT_EXPOSED|STORAGE_EXPOSED/);
  assert.equal(worker.active,active);assert.equal(worker.tabs.size,tabCount);assert.equal(worker.page.url(),url);assert.equal(worker.previewContext,null);
});
