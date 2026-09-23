// Render the built production React shell, never a real session or external site.
// Run with Node 24: node web/tests/browser-layout-browser.mjs
// Requires an existing Vite build, playwright-core and Chromium; installs nothing.
// Overrides: PLAYWRIGHT_MODULE, CHROMIUM_PATH, LAYOUT_OUTPUT.
import {createServer} from 'node:http';
import {readFile, mkdir, writeFile} from 'node:fs/promises';
import {resolve, dirname, extname} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';
import {createHash} from 'node:crypto';
import assert from 'node:assert/strict';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const output = resolve(process.env.LAYOUT_OUTPUT || `${root}/target/browser333-ux-layout`);
const {chromium} = await import(pathToFileURL(process.env.PLAYWRIGHT_MODULE || `${root}/voyage/browser/node_modules/playwright-core/index.mjs`).href);
const build = `${root}/web/public/build`;
const manifest = JSON.parse(await readFile(`${build}/manifest.json`, 'utf8'));
const entry = manifest['resources/react/main.tsx'];
const bootstrap = {tenantId:'layout-fixture',vessels:[{id:'c',vessel_id:'v',name:'Fixture Vessel'}],ticketUrl:'/console/ticket',legacyUrl:'/',connectionsUrl:'/connections',logoutUrl:'/console/logout'};
const html = `<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="csrf-token" content="fixture">${entry.css.map(css=>`<link rel="stylesheet" href="/build/${css}">`).join('')}</head><body><div id="helm-react" data-bootstrap='${JSON.stringify(bootstrap)}'></div><script type="module" src="/build/${entry.file}"></script></body></html>`;
const server = createServer(async (req,res)=>{
    try {
        if(req.url === '/') {res.setHeader('Content-Type','text/html');res.end(html);return;}
        const path = resolve(build, `.${new URL(req.url,'http://fixture').pathname.replace(/^\/build/,'')}`);
        if(!path.startsWith(build+'/')) {res.writeHead(404).end();return;}
        res.setHeader('Content-Type',extname(path)==='.css'?'text/css':'text/javascript');res.end(await readFile(path));
    } catch {res.writeHead(404).end();}
});
await new Promise(r=>server.listen(0,'127.0.0.1',r));
const origin = `http://127.0.0.1:${server.address().port}`;
await mkdir(output,{recursive:true});
const browser = await chromium.launch({executablePath:process.env.CHROMIUM_PATH || '/usr/bin/chromium',headless:true});
const report = {capturedAt:new Date().toISOString(),chromium:browser.version(),entry:entry.file,sha256:createHash('sha256').update(await readFile(`${build}/${entry.file}`)).digest('hex'),synthetic:true,viewports:[],failures:[]};
const check = (value, message) => {if(!value)report.failures.push(message);};
try {
 for(const viewport of [{width:1440,height:900},{width:390,height:844}]) {
    const label = viewport.width===1440?'desktop':'mobile';
    const context = await browser.newContext({viewport,reducedMotion:'reduce'});
    const page = await context.newPage();
    const errors=[];page.on('pageerror',e=>{errors.push(e.message);console.error('PAGE',e.message);});page.on('response',r=>{if(r.status()>=400)console.error('HTTP',r.status(),r.url());});
    await page.route('**/*',route=>{
        const url=route.request().url();
        if(url===origin+'/console/ticket')return route.fulfill({json:{url:'wss://fixture.invalid/v1/vessel/browser-socket',vessel_id:'v',token:'a'.repeat(64),expires_at_ms:Date.now()+120000}});
        return url.startsWith(origin+'/')?route.continue():route.abort();
    });
    await page.addInitScript(()=>{
        window.fixtureCommands=[];
        // Transport and media are synthetic. The App, DOM, CSS, browser layout,
        // controls and event handling are the unmodified production bundle.
        const binding={incarnation:'i',browser_id:'fixture-browser',attachment_id:'fixture-attachment',capture_epoch:1,controller_epoch:1};
        let mode='agent';
        class Socket extends EventTarget {
            readyState=0;protocol='voyage.vessel.v1';
            constructor(){super();queueMicrotask(()=>{this.readyState=1;this.dispatchEvent(new Event('open'));});}
            send(text){const f=JSON.parse(text);const emit=data=>queueMicrotask(()=>this.dispatchEvent(new MessageEvent('message',{data:JSON.stringify(data)})));
                if(f.type==='authenticate'){emit({type:'hello',protocol:1,vessel_id:'v',socket_id:'fixture-socket'});return;}
                if(['subscribe','unsubscribe'].includes(f.type))return;
                const c=f.request.command;window.fixtureCommands.push(c);let result;
                if(c.op==='capabilities')result={scope:'owner',vessel_id:'v'};
                else if(c.op==='catalogue')result=[{session_id:'a',incarnation:'i',name:'Browser layout fixture',state:'live',catalogue:{summary:{run_state:'idle'}}}];
                else if(c.op==='snapshot')result={session_id:'a',incarnation:'i',result:{session_id:'a',name:'Browser layout fixture',revision:1,observation_cursor:5,messages:Array.from({length:40},(_,i)=>({role:'assistant',content:`### Fixture observation ${i+1}\nSynthetic conversation content for scroll and composer layout verification. No personal browsing data.`,message_index:i})),run:{state:'idle',tool_previews:[{name:'host_browser'}]}}};
                else if(c.op==='decisions')result={session_id:'a',incarnation:'i',result:[]};
                else if(c.op==='host_browser'){
                    if(c.operation.action==='control'){mode=c.operation.mode;binding.controller_epoch++;}
                    result={session_id:'a',incarnation:'i',result:{status:{available:!window.fixtureUnavailable,running:!window.fixtureUnavailable,mode:window.fixtureUnavailable?null:mode,binding:window.fixtureUnavailable?null:{...binding},controller:mode==='private'?binding.attachment_id:null,input_sequence:0,page:{url:'https://fixture.invalid/',title:'Synthetic fixture'},tabs:[]},value:c.operation.action==='signal'?{type:'offer',sdp:'synthetic'}:undefined}};
                } else throw Error(`Unexpected fixture operation ${c.op}`);
                emit({type:'reply',request_id:f.request_id,response:{protocol:1,outcome_unknown:false,result}});
            }
            close(){this.readyState=3;this.dispatchEvent(new Event('close'));}
        }
        window.WebSocket=Socket;
        window.RTCPeerConnection=class {
            iceGatheringState='complete';localDescription=null;
            async setRemoteDescription(){const canvas=document.createElement('canvas');canvas.width=960;canvas.height=540;const ctx=canvas.getContext('2d');ctx.fillStyle='#153245';ctx.fillRect(0,0,960,540);ctx.fillStyle='#fff';ctx.font='28px sans-serif';ctx.fillText('Synthetic browser video — no external site',35,80);this.stream=canvas.captureStream(1);setTimeout(()=>this.ontrack?.({track:this.stream.getVideoTracks()[0],streams:[this.stream]}),20);}
            async createAnswer(){return {type:'answer',sdp:'synthetic-answer'};}
            async setLocalDescription(v){this.localDescription=v;}
            close(){this.stream?.getTracks().forEach(t=>t.stop());}
        };
    });
    await page.goto(origin);
    if(label==='mobile')await page.getByRole('button',{name:'Open voyage navigation',exact:true}).click();
    await page.locator('.voyage-card').click();
    const conversation=page.locator('.conversation:not([hidden])');
    const draft=conversation.locator('textarea').first();
    await draft.fill('Retained synthetic draft');
    await page.waitForTimeout(250);
    await conversation.locator('.transcript').evaluate(el=>{el.scrollTop=321;window.savedTranscript=el;window.savedDraft=document.querySelector('.conversation:not([hidden]) textarea');});
    const geometry=()=>page.evaluate(()=>{
        const rect=selector=>{const el=document.querySelector(selector);if(!el)return null;const r=el.getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom};};
        const transcript=document.querySelector('.conversation:not([hidden]) .transcript');
        const toggle=document.querySelector('.task-browser-action');const r=toggle.getBoundingClientRect();
        return {toggle:rect('.task-browser-action'),toggleHit:toggle.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)),panel:rect('.task-browser-panel'),composer:rect('.conversation:not([hidden]) form'),draft:rect('.conversation:not([hidden]) textarea'),transcript:rect('.conversation:not([hidden]) .transcript'),scrollTop:transcript.scrollTop,scrollHeight:transcript.scrollHeight,documentWidth:document.documentElement.scrollWidth,overflow:[...document.querySelectorAll('.conversation:not([hidden]) *')].filter(el=>el.getBoundingClientRect().right>innerWidth+1).map(el=>({tag:el.tagName,cls:el.className,text:el.textContent.slice(0,70),right:el.getBoundingClientRect().right})),primary:rect('.browser-primary'),privacy:rect('.browser-privacy'),activeLabel:document.activeElement.getAttribute('aria-label'),sameTranscript:transcript===window.savedTranscript,sameDraft:document.querySelector('.conversation:not([hidden]) textarea')===window.savedDraft};
    });
    const before=await geometry();
    await page.screenshot({path:`${output}/${label}-closed.png`});
    await page.getByRole('button',{name:'Browser',exact:true}).click();
    await page.waitForFunction(()=>document.querySelector('.host-browser-viewer')?.dataset.state==='agent');
    await page.waitForTimeout(150);
    const opened=await geometry();
    await page.screenshot({path:`${output}/${label}-open.png`});
    await page.getByRole('button',{name:'Take control privately',exact:true}).click();
    await page.waitForFunction(()=>document.querySelector('.host-browser-viewer')?.dataset.state==='private');
    const privateState=await geometry();
    const privateLabel=await page.locator('.browser-primary').textContent();
    const privateAccessibleLabel=await page.locator('.browser-primary').getAttribute('aria-label');
    await page.screenshot({path:`${output}/${label}-private.png`});
    await page.getByLabel('More browser options',{exact:true}).click();
    await page.getByRole('button',{name:'Capture and annotate',exact:true}).click();
    const editor=page.getByRole('dialog',{name:'Capture viewer image',exact:true});await editor.waitFor();
    check(await editor.getByRole('button',{name:'Add to message',exact:true}).isDisabled(),`${label}: capture needs disclosure confirmation`);
    await editor.getByRole('checkbox').check();await editor.getByRole('button',{name:'Add to message',exact:true}).click();await editor.waitFor({state:'hidden'});
    check(await conversation.locator('img').count()>0,`${label}: capture did not reach draft`);
    check(!(await page.evaluate(()=>window.fixtureCommands)).some(c=>['submit','submit_content'].includes(c.op)),`${label}: capture sent without Send`);

    await page.getByLabel('More browser options',{exact:true}).click();
    await page.screenshot({path:`${output}/${label}-more.png`});
    await page.getByLabel('More browser options',{exact:true}).click();
    // Record keyboard path through the mobile focus trap, including collapsed More.
    await page.locator('.task-browser-panel').focus();
    const focus=[];
    for(let i=0;i<12;i++){await page.keyboard.press('Tab');focus.push(await page.evaluate(()=>({tag:document.activeElement.tagName,label:document.activeElement.getAttribute('aria-label'),text:document.activeElement.textContent?.slice(0,60)})));}
    await page.keyboard.press('Escape');
    const focusAfterVideoEscape=await page.evaluate(()=>document.activeElement.tagName);
    await page.locator('.task-browser-panel').focus();
    await page.keyboard.press('Escape');
    await page.waitForTimeout(100);
    const escapeClosed=await page.locator('.task-browser-panel').count()===0;
    if(!escapeClosed)await page.getByRole('button',{name:'Close browser viewer',exact:true}).click();
    await page.locator('.task-browser-panel').waitFor({state:'detached'});
    const closed=await geometry();
    await page.screenshot({path:`${output}/${label}-after-close.png`});
    const retained=await draft.inputValue();
    const commands=await page.evaluate(()=>window.fixtureCommands.filter(c=>c.op==='host_browser').map(c=>c.operation.action));
    check(escapeClosed,`${label}: Escape from panel did not close`);
    check(before.toggleHit,`${label}: Browser toggle not hit-testable`);
    check(before.toggle.right<=viewport.width && before.toggle.y>=0,`${label}: Browser toggle outside viewport`);
    check(retained==='Retained synthetic draft' && closed.sameDraft && closed.sameTranscript,`${label}: draft/transcript DOM not retained`);
    check(closed.scrollTop===before.scrollTop,`${label}: close changed scroll ${before.scrollTop} → ${closed.scrollTop}`);
    check(closed.documentWidth===viewport.width,`${label}: document horizontally overflows`);
    if(label==='desktop')check(opened.composer.right<=opened.panel.x && opened.composer.bottom<=viewport.height,`${label}: split covers composer or composer below viewport`);
    else check(opened.panel.x===0 && opened.panel.y===0 && opened.panel.width===viewport.width && opened.panel.height===viewport.height,'mobile: overlay does not fit viewport');
    check(privateLabel==='Return to agent','Private primary label did not change');
    check(privateAccessibleLabel===privateLabel,`${label}: private visible/accessibility labels disagree (${privateLabel} / ${privateAccessibleLabel})`);
    check(commands.includes('detach')&&!commands.includes('close'),`${label}: close did not detach or stopped browser`);
    check(errors.length===0,`${label}: page errors: ${errors.join('; ')}`);
    await page.evaluate(()=>{window.fixtureUnavailable=true;});
    await page.getByRole('button',{name:'Browser',exact:true}).click();
    await page.waitForTimeout(150);
    const emptyState=await page.locator('.task-browser-panel').innerText();
    await page.screenshot({path:`${output}/${label}-unavailable.png`});
    report.viewports.push({emptyState,label,viewport,before,opened,privateState,closed,privateLabel,privateAccessibleLabel,retained,escapeClosed,focusAfterVideoEscape,focus,commands,errors});
    await context.close();
 }
} finally {
 await writeFile(`${output}/measurements.json`,JSON.stringify(report,null,2)+'\n');
 await browser.close();await new Promise(r=>server.close(r));
}
console.log(JSON.stringify(report,null,2));
assert.equal(report.failures.length,0,report.failures.join('\n'));
