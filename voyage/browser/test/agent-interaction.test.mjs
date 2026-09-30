import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import {randomUUID} from 'node:crypto';
import {Worker} from '../worker.mjs';

test('grounded agent paging, native input, child frames, stale targets and private fences',{timeout:60000},async t=>{
  const started=performance.now();let stage='initialization';
  const mark=value=>{stage=value;t.diagnostic(JSON.stringify({stage,elapsed_ms:Math.round(performance.now()-started)}));};
  const root=await fs.mkdtemp(path.join(os.tmpdir(),'browser-agent-379-')),w=new Worker(),sockets=new Set();
  const child=http.createServer((req,res)=>{res.setHeader('Content-Type','text/html');res.end('<label>Child name<input aria-label="Child name"></label><button onclick="this.textContent=\'Child applied\'">Apply child</button>');});
  await new Promise(r=>child.listen(0,'127.0.0.1',r));const childOrigin=`http://127.0.0.1:${child.address().port}`;
  const server=http.createServer();
  for(const site of [server,child])site.on('connection',s=>{sockets.add(s);s.on('close',()=>sockets.delete(s));});
  await new Promise(r=>server.listen(0,'127.0.0.1',r));const origin=`http://127.0.0.1:${server.address().port}`;
  // Same-origin child avoids recursively serving the full parent fixture.
  server.on('request',(req,res)=>{res.setHeader('Content-Type','text/html');if(req.url==='/same'){res.end('<button>Same child</button>');return;}res.end(`<title>Grounded forms</title><body><input aria-label="Name"><select aria-label="Choice"><option value="a">Alpha</option><option value="b">Beta</option></select><input type="checkbox" aria-label="Agree"><button ondblclick="this.textContent='Twice'">Double</button><div draggable="true" role="button" aria-label="Drag">Drag</div><button ondragover="event.preventDefault()" ondrop="event.preventDefault();this.textContent='Dropped'">Drop</button><iframe src="${childOrigin}"></iframe><iframe src="/same"></iframe>${'<button>Page item</button>'.repeat(140)}<p>${'large readable text '.repeat(1200)}</p>`);});
  t.after(async()=>{t.diagnostic(JSON.stringify({stage,cleanup:'started',elapsed_ms:Math.round(performance.now()-started)}));await w.dispose();for(const s of sockets)s.destroy();await Promise.all([new Promise(r=>server.close(r)),new Promise(r=>child.close(r))]);await fs.rm(root,{recursive:true,force:true});t.diagnostic(JSON.stringify({stage,cleanup:'observed',elapsed_ms:Math.round(performance.now()-started)}));});
  const request=(action,id=randomUUID())=>w.request({id,op:'agent',...w.status(),action});
  const act=async action=>{const r=await request(action);assert.equal(r.ok,true,JSON.stringify(r));return r.result.value;};
  const call=async(op,args={})=>{const r=await w.request({id:randomUUID(),op,...w.status(),...args});assert.equal(r.ok,true,JSON.stringify(r));return r.result.value;};
  await call('init',{config:{root,executable:process.env.CHROMIUM||'/usr/bin/chromium',public_web:false,origins:[origin,childOrigin].map(origin=>({origin,private_network:true})),width:1280,height:900}});await call('open');await act({kind:'navigate',url:origin});await w.page.waitForLoadState('load');
  mark('paging');const observe=()=>act({kind:'inspect'});let found=await observe();assert.equal(found.elements.length,64);assert.equal(found.next_offset,64);assert.equal(found.text_truncated,true);assert.equal(found.frames.length,2);
  const old=found.elements[0].ref;const paged=await act({kind:'inspect',offset:128,text_offset:16384});assert.equal(paged.next_offset,null);assert.match(paged.text,/large readable/);assert.equal((await request({kind:'fill',ref:old,text:'stale'})).error.code,'stale_reference');
  mark('native controls');const ref=async label=>(await observe()).elements.find(e=>e.text===label).ref;
  await act({kind:'fill',ref:await ref('Name'),text:'Voyage'});assert.equal(await w.page.locator('input').first().inputValue(),'Voyage');
  assert.equal((await request({kind:'key',ref:await ref('Name'),key:'eval(script)'})).error.state,'refused');
  assert.equal((await request({kind:'select',ref:await ref('Name'),value:'invented'})).error.code,'unsupported_select');
  assert.equal((await request({kind:'check',ref:await ref('Name'),checked:true})).error.code,'unsupported_check');
  assert.equal((await act({kind:'read',ref:await ref('Name')})).text,'');
  await act({kind:'key',ref:await ref('Name'),key:'Tab'});assert.equal(await w.page.evaluate(()=>document.activeElement.tagName),'SELECT');
  await act({kind:'select',ref:await ref('Choice'),value:'b'});assert.equal(await w.page.locator('select').inputValue(),'b');
  await act({kind:'check',ref:await ref('Agree'),checked:true});assert.equal(await w.page.locator('[type=checkbox]').isChecked(),true);
  await act({kind:'check',ref:await ref('Agree'),checked:false});assert.equal(await w.page.locator('[type=checkbox]').isChecked(),false);
  await act({kind:'double_click',ref:await ref('Double')});assert.equal(await w.page.locator('button').first().innerText(),'Twice');
  found=await observe();await act({kind:'drag',ref:found.elements.find(e=>e.text==='Drag').ref,target:found.elements.find(e=>e.text==='Drop').ref});assert.equal(await w.page.locator('button').nth(1).innerText(),'Dropped');
  mark('child frames');found=await observe();const frameId=found.frames.find(f=>f.url.startsWith(childOrigin)).frame;let inside=await act({kind:'inspect',frame:frameId});assert.equal(inside.elements[0].text,'Child name');
  await act({kind:'fill',ref:inside.elements[0].ref,text:'Frame input'});assert.equal(await w.page.frames().find(f=>f.url().startsWith(childOrigin)).locator('input').inputValue(),'Frame input');
  inside=await act({kind:'inspect',frame:frameId});const childRef=inside.elements[1].ref;
  await w.page.frames().find(f=>f.url().startsWith(childOrigin)).goto(childOrigin+'/changed');assert.equal((await request({kind:'click',ref:childRef})).error.code,'stale_reference');assert.equal((await request({kind:'inspect',frame:frameId})).error.code,'stale_frame');
  found=await observe();assert.match((await act({kind:'read',ref:found.elements.find(e=>e.text==='Twice').ref})).text,/Twice/);
  const changed=found.elements[0].ref;await w.page.locator('input').first().evaluate(e=>e.setAttribute('aria-label','Changed'));assert.equal((await request({kind:'fill',ref:changed,text:'invalid'})).error.code,'stale_reference');
  const id=randomUUID(),key={kind:'key',ref:await ref('Changed'),key:'Escape'};// Exact command identity must return its retained receipt without re-execution.
  const exact=await request(key,id);assert.equal(exact.ok,true);const repeated=await request(key,id);assert.equal(repeated.result.content_withheld,true);assert.equal(repeated.result.receipt.state,'completed');
  await w.page.evaluate(()=>{console.error('PRIVATE_SENTINEL');setTimeout(()=>{throw new Error('PRIVATE_SENTINEL');},0);});await new Promise(r=>setTimeout(r,50));const diagnostics=await act({kind:'diagnostics'});assert.ok(diagnostics.errors.console>=1);assert.ok(diagnostics.errors.page>=1);assert.doesNotMatch(JSON.stringify(diagnostics),/PRIVATE_SENTINEL/);
  mark('history and uncertain effect');await act({kind:'navigate',url:origin+'/next'});await act({kind:'history',direction:'back'});assert.equal(w.page.url(),origin+'/');await act({kind:'history',direction:'forward'});assert.equal(w.page.url(),origin+'/next');await act({kind:'history',direction:'reload'});
  await w.page.waitForLoadState('load');
  await w.page.evaluate(()=>{window.effects=0;document.querySelector('input').onkeydown=e=>{if(e.key==='Enter')window.effects++;};});
  const uncertainId=randomUUID(),uncertainAction={kind:'key',ref:await ref('Name'),key:'Enter'},perform=w.perform.bind(w);
  w.perform=async(a,stamp)=>{const result=await perform(a,stamp);if(a.kind==='key'&&a.key==='Enter')throw new Error('synthetic lost effect response');return result;};
  const uncertain=await request(uncertainAction,uncertainId);assert.deepEqual(uncertain.error,{code:'outcome_unknown',state:'unknown'});
  const noReplay=await request(uncertainAction,uncertainId);assert.equal(noReplay.result.receipt.state,'unknown');assert.equal(await w.page.evaluate(()=>window.effects),1);w.perform=perform;
  mark('private fences');const viewer=randomUUID();await call('join',{viewer});await call('control',{viewer,mode:'private'});await w.page.locator('input').first().fill('HUMAN_PRIVATE');
  for(const kind of ['inspect','diagnostics','history','key','select','check','double_click','drag','read']){const r=await request({kind});assert.equal(r.error.code,'agent_fenced');assert.equal(r.result,undefined);}
  await call('control',{viewer,mode:'agent'});assert.deepEqual((await act({kind:'diagnostics'})).errors,{console:0,page:0});
  mark('option bounds');await w.page.setContent('<select aria-label="Large choice">'+Array.from({length:80},(_,i)=>`<option value="${i}-${'界'.repeat(250)}">${'界'.repeat(250)}</option>`).join('')+'</select>');
  const large=await observe();assert.equal(large.elements.length,1);assert.equal(large.elements[0].options_truncated,true);assert.ok(large.elements[0].options.length>0);assert.equal(large.next_offset,null);assert.ok(Buffer.byteLength(JSON.stringify(large.elements))<32768);assert.ok(Buffer.byteLength(JSON.stringify(large))<128*1024);
  // A shadow control before a light-DOM control must not shift a separately
  // queried locator's indices. The exact captured list supplies both references.
  mark('shadow controls');await w.page.setContent('<div id="shadow"></div><div id="late"></div><button id="light">Light action</button>');
  await w.page.evaluate(()=>{
    const shadow=document.querySelector('#shadow').attachShadow({mode:'open'});
    shadow.innerHTML='<span id="name">Shadow label</span><input aria-labelledby="name"><button onclick="this.textContent=\'Shadow applied\'">Shadow action</button><div id="nested"></div>';
    shadow.querySelector('#nested').attachShadow({mode:'open'}).innerHTML='<button onclick="this.textContent=\'Nested applied\'">Nested action</button>';
  });
  const shadowObserved=await observe();assert.equal(shadowObserved.control_total,4);assert.equal(shadowObserved.controls_truncated,false);
  assert.deepEqual(shadowObserved.elements.map(e=>e.text),['Shadow label','Shadow action','Nested action','Light action']);
  assert.equal(shadowObserved.elements.find(e=>e.text==='Shadow action').obscured,false);
  const shadowPage=await act({kind:'inspect',offset:2,limit:1});assert.equal(shadowPage.elements[0].text,'Nested action');assert.equal(shadowPage.next_offset,3);
  await act({kind:'click',ref:await ref('Shadow action')});assert.equal(await w.page.locator('#shadow button').first().innerText(),'Shadow applied');
  await act({kind:'click',ref:await ref('Nested action')});assert.equal(await w.page.locator('#nested button').innerText(),'Nested applied');
  let lightRef=await ref('Light action');
  await w.page.evaluate(()=>document.querySelector('#shadow').shadowRoot.querySelector('#name').textContent='Changed shadow label');
  assert.equal((await request({kind:'click',ref:lightRef})).error.code,'stale_reference','shadow mutations invalidate the whole observation');
  lightRef=await ref('Light action');
  await w.page.evaluate(()=>document.querySelector('#late').attachShadow({mode:'open'}).innerHTML='<button>New shadow action</button>');
  assert.equal((await request({kind:'click',ref:lightRef})).error.code,'stale_reference','new shadow roots are discovered even without a light-DOM mutation');
  assert.ok((await observe()).elements.some(e=>e.text==='New shadow action'));
  mark('shadow depth bound');await w.page.setContent('<div id="deep"></div><button>Reachable light action</button>');
  await w.page.evaluate(()=>{let host=document.querySelector('#deep');for(let i=0;i<33;i++){const root=host.attachShadow({mode:'open'});host=document.createElement('div');root.append(host);}host.innerHTML='<button>Over depth action</button>';});
  const deep=await observe();assert.equal(deep.controls_truncated,true);assert.deepEqual(deep.elements.map(e=>e.text),['Reachable light action']);assert.ok(deep.unsupported.some(value=>value.includes('32 open shadow levels')));
  // Keep the traversal workload separate from Chromium's unrelated huge inline
  // formatting cost. Hidden nodes still consume the exact document node budget;
  // the visible button after them must not gain a grounded reference.
  mark('wide document setup');await w.page.setContent('<button>Early control</button><main hidden></main>');
  await w.page.evaluate(()=>{const nodes=document.createDocumentFragment();for(let i=0;i<100001;i++)nodes.append(document.createElement('span'));document.querySelector('main').append(nodes);const button=document.createElement('button');button.textContent='Outside node budget';document.body.append(button);
    window.unboundedReads=0;
    Object.defineProperty(document.body,'innerText',{configurable:true,get(){window.unboundedReads++;throw new Error('oversized text read must not run');}});
    document.querySelector('button').getBoundingClientRect=()=>{window.unboundedReads++;throw new Error('oversized control geometry must not run');};
  });
  mark('wide document observation');const wide=await observe();assert.equal(wide.controls_truncated,true);assert.equal(wide.control_total,1);assert.equal(wide.next_offset,null);assert.equal(wide.text_truncated,true);assert.equal(wide.elements.length,0);assert.ok(wide.unsupported.some(value=>value.includes('control references withheld')));assert.equal(await w.page.evaluate(()=>window.unboundedReads),0);mark('receipt audit');
  const receipts=await fs.readdir(path.join(root,'receipts'));for(const file of receipts)assert.doesNotMatch(await fs.readFile(path.join(root,'receipts',file),'utf8'),/HUMAN_PRIVATE|PRIVATE_SENTINEL|Frame input/);
});
