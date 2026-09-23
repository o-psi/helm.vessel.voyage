// Real Chromium layout and interaction journey against the production viewer
// module. Transport/media are synthetic, so this does not claim remote access.
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
</style></head><body><div class="workspace"><aside class="conversation"><h1>Example voyage</h1><div class="messages">Agent and user conversation stays here.</div><textarea>Unsent draft stays here.</textarea></aside><main id="viewer"></main></div><script type="module">
import {mountBrowserViewer} from '/helm/browser-view/viewer.mjs';
const nil='00000000-0000-0000-0000-000000000000';
const binding={incarnation:'incarnation',browser_id:'browser',attachment_id:nil,tab_id:'tab',document_epoch:1,viewport_epoch:1,controller_epoch:1,capture_epoch:1};
const state={available:true,running:true,mode:'agent',controller:null,binding,tabs:['tab'],viewport:{width:1280,height:720},page:{url:'https://example.com/',title:'Example Domain',can_go_back:false,can_go_forward:false},tab_details:[{id:'tab',title:'Example Domain'}],input_sequence:0};
window.fixtureCommands=[];
const copy=()=>structuredClone(state);
const transport=async operation=>{window.fixtureCommands.push(operation);
  if(operation.action==='attach')state.binding.attachment_id=operation.binding.attachment_id;
  if(operation.action==='control'){state.mode=operation.mode;state.controller=operation.mode==='agent'?null:state.binding.attachment_id;state.binding.controller_epoch++;state.binding.capture_epoch++;}
  if(operation.action==='input'){
    state.input_sequence=operation.sequence;
    const action=operation.input;
    if(action.type==='resize'){state.viewport={width:action.width,height:action.height};state.binding.viewport_epoch++;canvas.width=action.width;canvas.height=action.height;draw();}
    if(action.type==='navigate'){state.page={url:action.url,title:'Navigated page',can_go_back:true,can_go_forward:false};state.tab_details=[{id:'tab',title:'Navigated page'}];}
  }
  return {status:copy(),value:operation.action==='signal'&&operation.signal.type==='request_offer'?{type:'offer',sdp:'fixture'}:null};
};
const canvas=document.createElement('canvas');canvas.width=1280;canvas.height=720;
const context=canvas.getContext('2d');
const draw=()=>{const narrow=canvas.width<600,x=narrow?20:120;context.fillStyle='#fff';context.fillRect(0,0,canvas.width,canvas.height);context.fillStyle='#111';context.font=narrow?'26px sans-serif':'48px sans-serif';context.fillText('Example Domain',x,narrow?65:125);context.font=narrow?'16px sans-serif':'27px sans-serif';context.fillText(narrow?'A responsive synthetic page.':'This is a synthetic browser page for layout testing.',x,narrow?108:200);};draw();setInterval(draw,300);
const stream=canvas.captureStream(4);
const peer=()=>({iceGatheringState:'complete',connectionState:'connected',localDescription:null,async setRemoteDescription(){setTimeout(()=>this.ontrack?.({track:stream.getVideoTracks()[0],streams:[stream]}),20);},async createAnswer(){return {type:'answer',sdp:'fixture-answer'};},async setLocalDescription(value){this.localDescription=value;},close(){}});
window.viewer=mountBrowserViewer(document.querySelector('#viewer'),{transport,peer,context:()=>({incarnation:'incarnation',revision:1})});
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
const report={origin,viewports:[],failures:[]};
try{
    for(const viewport of [{width:1280,height:720},{width:390,height:844}]){
        const page=await browser.newPage({viewport});
        const errors=[];page.on('pageerror',error=>errors.push(error.message));
        await page.goto(origin);
        await page.locator('.browser-next[data-state="agent"]').waitFor({timeout:15000});
        await page.screenshot({path:`${output}/${viewport.width}-watching.png`});
        await page.getByRole('button',{name:'Take control privately'}).click();
        await page.locator('.browser-next[data-state="private"]').waitFor({timeout:15000});
        await page.waitForFunction(()=>window.fixtureCommands.some(c=>c.action==='input'&&c.input?.type==='resize'),{timeout:5000}).catch(async error=>{
            console.error('RESIZE STATE',await page.evaluate(()=>({phase:window.viewer.session.phase,controls:window.viewer.session.controls,canInput:window.viewer.session.canInput,streaming:window.viewer.session.streaming,busy:window.viewer.session.busy,issue:window.viewer.session.issue,viewport:window.viewer.session.status.viewport,scroll:[document.querySelector('.browser-next-scroll').clientWidth,document.querySelector('.browser-next-scroll').clientHeight],commands:window.fixtureCommands.map(c=>({action:c.action,input:c.input?.type}))})));
            throw error;
        });
        await page.screenshot({path:`${output}/${viewport.width}-private.png`});
        const geometry=await page.evaluate(()=>{
            const rect=selector=>{const r=document.querySelector(selector).getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom};};
            return {viewport:{width:innerWidth,height:innerHeight},documentWidth:document.documentElement.scrollWidth,viewer:rect('.browser-next'),stage:rect('.browser-next-stage'),video:rect('video'),address:rect('.browser-next-address'),control:rect('.browser-next-primary'),remote:window.viewer.session.status.viewport};
        });
        await page.getByRole('textbox',{name:'Website address'}).fill('example.org');
        await page.getByRole('button',{name:'Go to address'}).click();
        await page.waitForFunction(()=>window.fixtureCommands.some(c=>c.action==='input'&&c.input?.type==='navigate'));
        if(geometry.documentWidth>viewport.width)report.failures.push(`${viewport.width}: horizontal overflow`);
        if(geometry.stage.height<240)report.failures.push(`${viewport.width}: browser too short`);
        if(geometry.remote.width>geometry.stage.width+30)report.failures.push(`${viewport.width}: remote viewport remains too wide`);
        if(errors.length)report.failures.push(`${viewport.width}: ${errors.join('; ')}`);
        report.viewports.push({geometry,errors,commands:await page.evaluate(()=>window.fixtureCommands.map(c=>c.action))});
        await page.close();
    }
} finally {await writeFile(`${output}/report.json`,JSON.stringify(report,null,2));await browser.close();await new Promise(resolve=>server.close(resolve));}
console.log(JSON.stringify(report,null,2));
assert.deepEqual(report.failures,[]);
