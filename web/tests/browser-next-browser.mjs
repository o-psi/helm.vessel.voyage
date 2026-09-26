// Real Chromium replay and interaction journey against the shared viewer.
import {createServer} from 'node:http';
import {readFile,mkdir,writeFile} from 'node:fs/promises';
import {resolve,dirname,extname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {chromium} from '../../voyage/browser/node_modules/playwright-core/index.mjs';
import assert from 'node:assert/strict';

const repo=resolve(dirname(fileURLToPath(import.meta.url)),'../..');
const output=resolve(process.env.BROWSER_NEXT_OUTPUT||`${repo}/target/browser-next`);
const mime={'.mjs':'text/javascript','.css':'text/css','.html':'text/html'};
const html=`<!doctype html><html><head><meta name="viewport" content="width=device-width, initial-scale=1"><link rel="stylesheet" href="/helm/browser-view/viewer.css"><style>
body{margin:0;font:14px system-ui;color:#182638;background:#f4f7fa}*{box-sizing:border-box}
.workspace{height:100dvh;min-width:0;display:grid;grid-template-columns:minmax(260px,28%) minmax(0,1fr)}
.conversation{min-width:0;min-height:0;display:flex;flex-direction:column;padding:20px;border-right:1px solid #cdd7e2}
.conversation h1{font-size:18px}.conversation .messages{flex:1;overflow:auto}.conversation textarea{width:100%;height:75px}
#viewer{height:100%;min-width:0;min-height:0;padding:8px}
@media(max-width:750px){.workspace{display:block}.conversation{display:none}#viewer{padding:0}}
</style><script src="/voyage/browser/rrweb-vendor.mjs"></script></head><body><article id="site"><h1>Example Domain</h1><p>A responsive synthetic page.</p><button id="site-button">Open details</button><label>Your name <input id="site-input"></label></article><script type="module">
import {mountBrowserViewer} from '/helm/browser-view/viewer.mjs';
const child=document.createElement('iframe');child.width='400';child.height='220';child.srcdoc='<html><head><script src="/voyage/browser/rrweb-vendor.mjs"><\\/script></head><body><h2>Child form</h2><button>Frame action</button><label>Frame code <input></label><canvas width="80" height="35"></canvas></body></html>';
document.querySelector('#site').append(child);
await new Promise(resolve=>child.addEventListener('load',resolve,{once:true}));
const childCanvas=child.contentDocument.querySelector('canvas');childCanvas.getContext('2d').fillRect(0,0,80,35);
const nested=child.contentDocument.createElement('iframe');nested.width='180';nested.height='90';nested.srcdoc='<html><head><script src="/voyage/browser/rrweb-vendor.mjs"><\\/script></head><body><h3>Nested form</h3><button>Nested action</button><canvas width="50" height="20"></canvas></body></html>';
child.contentDocument.body.append(nested);
await new Promise(resolve=>nested.addEventListener('load',resolve,{once:true}));
const nestedCanvas=nested.contentDocument.querySelector('canvas');nestedCanvas.getContext('2d').fillRect(0,0,50,20);
const nestedEvents=[];const stopNested=nested.contentWindow.rrweb.record({emit:event=>nestedEvents.push(event),inlineStylesheet:true});stopNested();
const childEvents=[];const stopChild=child.contentWindow.rrweb.record({emit:event=>childEvents.push(event),inlineStylesheet:true});stopChild();
const recorded=[];const stop=rrweb.record({emit:event=>recorded.push(event),inlineStylesheet:true});stop();
const bytes=new TextEncoder();
const encoded=async events=>{const compressed=await new Response(new Blob([bytes.encode(JSON.stringify(events))]).stream().pipeThrough(new CompressionStream('gzip'))).arrayBuffer();const raw=new Uint8Array(compressed);let binary='';for(let i=0;i<raw.length;i+=16384)binary+=String.fromCharCode(...raw.subarray(i,i+16384));return btoa(binary);};
const payload=await encoded(recorded);
const childPayload=await encoded(childEvents);
const nestedPayload=await encoded(nestedEvents);
const emptyPayload=await encoded([]);
const childHost=rrweb.record.mirror.getId(child);
const nestedHost=child.contentWindow.rrweb.record.mirror.getId(nested);
const childVisual={id:child.contentWindow.rrweb.record.mirror.getId(childCanvas),data_base64:childCanvas.toDataURL('image/jpeg',0.5).split(',')[1],version:1};
const nestedVisual={id:nested.contentWindow.rrweb.record.mirror.getId(nestedCanvas),data_base64:nestedCanvas.toDataURL('image/jpeg',0.5).split(',')[1],version:1};
const childId='11111111-1111-4111-8111-111111111111';
const nestedId='22222222-2222-4222-8222-222222222222';
const nil='00000000-0000-0000-0000-000000000000';
const binding={incarnation:'incarnation',browser_id:'browser',attachment_id:nil,tab_id:'tab',document_epoch:1,viewport_epoch:1,controller_epoch:1,capture_epoch:1};
const state={available:true,running:true,mode:'agent',controller:null,binding,tabs:['tab'],viewport:{width:1280,height:720},page:{url:'https://example.com/',title:'Example Domain',can_go_back:false,can_go_forward:false},tab_details:[{id:'tab',title:'Example Domain'}],input_sequence:0};
window.fixtureCommands=[];
const transport=async operation=>{
 window.fixtureCommands.push(operation);
 if(operation.action==='attach')state.binding.attachment_id=operation.binding.attachment_id;
 if(operation.action==='control'){state.mode=operation.mode;state.controller=operation.mode==='agent'?null:state.binding.attachment_id;state.binding.controller_epoch++;state.binding.capture_epoch++;}
 if(operation.action==='input'){state.input_sequence=operation.sequence;const action=operation.input;
  if(action.type==='resize'){state.viewport={width:action.width,height:action.height};state.binding.viewport_epoch++;}
  if(action.type==='navigate'){state.page={url:action.url,title:'Navigated page',can_go_back:true,can_go_forward:false};state.tab_details=[{id:'tab',title:'Navigated page'}];}
 }
 return {status:structuredClone(state),value:operation.action==='mirror'?{encoding:'gzip',data_base64:payload,reset:true,cursor:recorded.length,latest:recorded.length,visuals:[],frames:[{frame_id:childId,parent_frame_id:null,host_node_id:childHost,encoding:'gzip',data_base64:childPayload,reset:true,cursor:childEvents.length,visuals:[childVisual]},{frame_id:nestedId,parent_frame_id:childId,host_node_id:nestedHost,encoding:'gzip',data_base64:nestedPayload,reset:true,cursor:nestedEvents.length,visuals:[nestedVisual]}]}:null};
};
const workspace=document.createElement('div');workspace.className='workspace';workspace.innerHTML='<aside class="conversation"><h1>Example voyage</h1><div class="messages">Conversation</div><textarea>Unsent draft</textarea></aside><main id="viewer"></main>';
document.body.replaceChildren(workspace);
window.viewer=mountBrowserViewer(document.querySelector('#viewer'),{transport,context:()=>({incarnation:'incarnation',revision:1})});
window.fixtureReparent=async()=>{
 const session=window.viewer.session,nestedRoot=session.frames.get(nestedId).root;
 await session.updateFrames([
  {frame_id:childId,parent_frame_id:null,host_node_id:childHost,encoding:'gzip',data_base64:childPayload,reset:true,cursor:childEvents.length,visuals:[childVisual]},
  {frame_id:nestedId,parent_frame_id:childId,host_node_id:nestedHost,encoding:'gzip',data_base64:emptyPayload,reset:false,cursor:nestedEvents.length,visuals:[nestedVisual]},
 ]);
 return nestedRoot===session.frames.get(nestedId).root&&nestedRoot.parentNode===session.frames.get(childId).root;
};
</script></body></html>`;
const server=createServer(async(req,res)=>{
 if(req.url==='/'){res.setHeader('Content-Type','text/html');res.end(html);return;}
 const path=resolve(repo,`.${new URL(req.url,'http://fixture').pathname}`);
 if(!path.startsWith(repo+'/')){res.writeHead(404).end();return;}
 try{res.setHeader('Content-Type',mime[extname(path)]||'application/octet-stream');res.end(await readFile(path));}
 catch{res.writeHead(404).end();}
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const origin=`http://127.0.0.1:${server.address().port}`;
const browser=await chromium.launch({executablePath:process.env.CHROMIUM_PATH||'/usr/bin/chromium',headless:true});
await mkdir(output,{recursive:true});
const report={viewports:[],failures:[]};
try{
 for(const viewport of [{width:1280,height:720},{width:390,height:844}]){
  const page=await browser.newPage({viewport});const errors=[];page.on('pageerror',error=>errors.push(error.message));
  await page.goto(origin);
  await page.locator('.browser-next[data-state="agent"]').waitFor({timeout:15000});
  const frame=page.frameLocator('.browser-next-mirror iframe');
  await frame.getByRole('heading',{name:'Example Domain'}).waitFor({timeout:5000}).catch(async error=>{
   console.error('REPLAY',await page.evaluate(()=>({phase:window.viewer?.session.phase,issue:window.viewer?.session.issue,streaming:window.viewer?.session.streaming,commands:window.fixtureCommands?.map(c=>c.action),mirror:document.querySelector('.browser-next-mirror')?.outerHTML?.slice(0,1000),frame:document.querySelector('.browser-next-mirror iframe')?.contentDocument?.body?.outerHTML?.slice(0,1000)})));
   throw error;
  });
  assert.equal(await frame.locator('meta[http-equiv="Content-Security-Policy"]').count(),1,'replay CSP must survive full snapshot reconstruction');
  await page.getByRole('button',{name:'Take control privately'}).click();
  await page.locator('.browser-next[data-state="private"]').waitFor({timeout:15000});
  await frame.getByRole('button',{name:'Open details'}).click();
  await page.waitForFunction(()=>window.fixtureCommands.some(c=>c.action==='input'&&c.input?.type==='click'&&c.input.node_id>0),null,{timeout:5000}).catch(async error=>{
   console.error('INPUT',JSON.stringify(await page.evaluate(()=>({phase:window.viewer?.session.phase,canInput:window.viewer?.session.canInput,commands:window.fixtureCommands.filter(c=>c.action==='input').map(c=>c.input),frames:[...document.querySelectorAll('.browser-next-mirror iframe')].map(f=>f.contentDocument?.body?.innerHTML?.slice(0,1000))})),null,2));
   throw error;
  });
  await frame.getByRole('textbox',{name:'Your name'}).fill('Voyager');
  await page.waitForFunction(()=>window.fixtureCommands.some(c=>c.action==='input'&&c.input?.type==='fill'&&c.input.text==='Voyager'));
  const childFrame=page.frameLocator('.browser-next-frames > .browser-next-frame > .replayer-wrapper > iframe');
  await childFrame.getByRole('heading',{name:'Child form'}).waitFor({timeout:5000});
  await childFrame.getByRole('button',{name:'Frame action'}).click();
  await page.waitForFunction(()=>window.fixtureCommands.some(c=>c.action==='input'&&c.input?.type==='frame_element'&&c.input.input?.type==='click'&&c.input.frame_id==='11111111-1111-4111-8111-111111111111'));
  await childFrame.getByRole('textbox',{name:'Frame code'}).fill('Frame secret');
  await page.waitForFunction(()=>window.fixtureCommands.some(c=>c.action==='input'&&c.input?.type==='frame_element'&&c.input.input?.type==='fill'&&c.input.input.text==='Frame secret'));
  const nestedFrame=page.frameLocator('.browser-next-frame .browser-next-frame iframe');
  await nestedFrame.getByRole('heading',{name:'Nested form'}).waitFor({timeout:5000});
  await nestedFrame.getByRole('button',{name:'Nested action'}).click();
  await page.waitForFunction(()=>window.fixtureCommands.some(c=>c.action==='input'&&c.input?.type==='frame_element'&&c.input.frame_id==='22222222-2222-4222-8222-222222222222'));
  await page.waitForFunction(()=>[...document.querySelectorAll('.browser-next-frame-visual')].length===2&&[...document.querySelectorAll('.browser-next-frame-visual')].every(image=>image.complete&&image.naturalWidth>0));
  assert.equal(await page.evaluate(()=>window.fixtureReparent()),true,'nested replay survives a parent snapshot rebuild');
  const geometry=await page.evaluate(()=>({width:document.documentElement.scrollWidth,stage:document.querySelector('.browser-next-stage').getBoundingClientRect().height,mirror:!!document.querySelector('.browser-next-mirror .replayer-wrapper'),commands:window.fixtureCommands.map(c=>({action:c.action,input:c.input?.type}))}));
  if(geometry.width>viewport.width)report.failures.push(`${viewport.width}: horizontal overflow`);
  if(geometry.stage<240)report.failures.push(`${viewport.width}: browser too short`);
  if(errors.length)report.failures.push(`${viewport.width}: ${errors.join('; ')}`);
  report.viewports.push({viewport,geometry});await page.screenshot({path:`${output}/${viewport.width}-private.png`});await page.close();
 }
}finally{await writeFile(`${output}/report.json`,JSON.stringify(report,null,2));await browser.close();await new Promise(resolve=>server.close(resolve));}
console.log(JSON.stringify(report,null,2));assert.deepEqual(report.failures,[]);
