// Real Chromium and maintained rrweb recorder/replayer; no external sites or providers.
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFile} from 'node:fs/promises';
import {createServer} from 'node:http';
import {gzipSync} from 'node:zlib';
import {chromium} from '../browser/node_modules/playwright-core/index.mjs';

const viewer=await readFile(new URL('../../helm/browser-view/viewer.mjs',import.meta.url));
const vendor=await readFile(new URL('../browser/rrweb-vendor.mjs',import.meta.url));
const replacement='url("data:image/svg+xml;base64,'+Buffer.from('<svg xmlns="http://www.w3.org/2000/svg" width="33" height="17"/>').toString('base64')+'")';
const source=`<!doctype html><style>body{margin:8px}.row{display:flex;gap:8px}canvas.css{width:120px;height:40px}.shrink{width:160px}.shrink canvas{min-width:0}#author{content:${replacement}}</style>
<div class="row"><canvas id="canvas" width="240" height="80"></canvas><video id="video" width="240" height="80"></video></div>
<canvas id="default"></canvas><canvas id="css" class="css" width="240" height="80"></canvas>
<div class="row shrink"><canvas id="shrink" width="240" height="80"></canvas><span>text</span></div>
<canvas id="author" width="240" height="80"></canvas><div id="shadow"></div><script>window.siteScript=true;document.querySelector('#shadow').attachShadow({mode:'open'}).innerHTML='<canvas id="shadowCanvas" width="70" height="20"></canvas>';</script>`;
const dimensions=()=>{
 const nodes=[...document.querySelectorAll('canvas,video'),...document.querySelector('#shadow').shadowRoot.querySelectorAll('canvas')];
 return nodes.map(node=>{const r=node.getBoundingClientRect();return {id:node.id,tag:node.tagName,left:r.left,top:r.top,width:r.width,height:r.height};});
};

test('script-free replay preserves canvas intrinsic layout, CSS sizing and live mutations',{timeout:20000},async()=>{
 const server=createServer((req,res)=>{res.setHeader('Content-Type',req.url==='/viewer.mjs'?'text/javascript':'text/html');res.end(req.url==='/viewer.mjs'?viewer:req.url==='/source'?source:'<!doctype html><div id="mirror"></div>');});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 let browser;
 try{
  browser=await chromium.launch({executablePath:process.env.CHROMIUM_PATH||'/usr/bin/chromium',headless:true,chromiumSandbox:true});
  const context=await browser.newContext({viewport:{width:800,height:600}}),task=await context.newPage(),helm=await context.newPage();
  const origin=`http://127.0.0.1:${server.address().port}`;
  await task.goto(`${origin}/source`);await task.addScriptTag({content:vendor.toString()});
  await task.evaluate(()=>{window.events=[];window.stopRecord=rrweb.record({emit:event=>events.push(event),inlineStylesheet:true,recordCanvas:false});});
  await helm.goto(origin);await helm.addScriptTag({content:vendor.toString()});
  await helm.evaluate(async()=>{
   const {BrowserConnection}=await import('/viewer.mjs');
   window.connection=new BrowserConnection({mirror:document.querySelector('#mirror'),transport:async()=>({status:connection.status,value:window.packet})});
   connection.status={available:true,running:true,binding:{browser_id:'browser',incarnation:'owner',attachment_id:'attachment',tab_id:'tab',document_epoch:1,capture_epoch:1,controller_epoch:1}};
  });
  let cursor=0;
  const transfer=async reset=>{
   const events=await task.evaluate(()=>events.splice(0));
   assert.ok(events.length>0);
   const packet={reset,cursor:++cursor,encoding:'gzip',data_base64:gzipSync(JSON.stringify(events)).toString('base64'),frames:[]};
   await helm.evaluate(async packet=>{window.packet=packet;await connection.pull();},packet);
   await helm.waitForFunction(()=>connection.streaming&&connection.replayer?.iframe.contentDocument.querySelector('#canvas'));
  };
  const equal=async()=>{
   const expected=await task.evaluate(dimensions);
   // rrweb's live timer and inert image decode finish asynchronously.
   await helm.waitForFunction(({expected,code})=>{
    const doc=connection.replayer.iframe.contentDocument;
    const read=new Function('document',`return (${code})();`);
    return JSON.stringify(read(doc))===JSON.stringify(expected);
   },{expected,code:dimensions.toString()},{timeout:2500});
   const result=await helm.evaluate(()=>({sandbox:connection.replayer.iframe.getAttribute('sandbox'),siteScript:connection.replayer.iframe.contentWindow.siteScript,canvasId:connection.replayer.getMirror().getId(connection.replayer.iframe.contentDocument.querySelector('#canvas'))}));
   assert.ok(!result.sandbox.includes('allow-scripts'));assert.equal(result.siteScript,undefined);assert.ok(result.canvasId>0);
   return result.canvasId;
  };
  await transfer(true);const nodeId=await equal();
  await task.evaluate(()=>{const c=document.querySelector('#canvas');c.width=180;c.height=60;c.style.border='2px solid red';const added=document.createElement('canvas');added.id='added';added.width=50;added.height=25;document.body.append(added);document.querySelector('#shadow').shadowRoot.querySelector('canvas').width=90;});
  await task.waitForTimeout(50);await transfer(false);assert.equal(await equal(),nodeId);
  // rrweb replaces the style attribute; replay sizing must survive it too.
  await task.evaluate(()=>{document.querySelector('#canvas').setAttribute('style','width: 90px; height: 30px');});
  await task.waitForTimeout(50);await transfer(false);assert.equal(await equal(),nodeId);
  await task.evaluate(replacement=>{document.querySelector('#canvas').setAttribute('style',`content:${replacement}`);},replacement);
  await task.waitForTimeout(50);await transfer(false);assert.equal(await equal(),nodeId);
  await task.evaluate(()=>{document.querySelector('#canvas').style.content='none';document.querySelector('#canvas').dataset.phase='authored-none';});
  await task.waitForTimeout(50);await transfer(false);
  // Wait for the incremental none mutation itself, even though its geometry is unchanged.
  await helm.waitForFunction(()=>connection.replayer.iframe.contentDocument.querySelector('#canvas').dataset.phase==='authored-none',null,{timeout:2500});
  assert.equal(await equal(),nodeId);
  const retiredContent=await helm.evaluate(async()=>{const canvas=connection.replayer.iframe.contentDocument.querySelector('#canvas');connection.clearMirror();canvas.removeAttribute('style');canvas.width=300;await new Promise(resolve=>setTimeout(resolve,0));return canvas.style.content;});
  assert.equal(retiredContent,'');assert.equal(await helm.locator('#mirror iframe').count(),0);
 }finally{await browser?.close();await new Promise(resolve=>server.close(resolve));}
});
