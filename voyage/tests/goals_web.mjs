// Built Helm Web against a real disposable Vessel; invoked by goals.py --web-root.
// Only the socket destination is redirected to a loopback TLS proxy. Protocol,
// owner credentials, snapshots, mutations, receipts and events are real.
import {createServer} from 'node:https';
import {connect} from 'node:net';
import {readFile,writeFile} from 'node:fs/promises';
import {resolve,extname} from 'node:path';
import {pathToFileURL} from 'node:url';
import assert from 'node:assert/strict';
const config=JSON.parse(await readFile(process.argv[2],'utf8'));
const root=resolve(config.webRoot), build=resolve(root,'public/build');
const {chromium}=await import(pathToFileURL(`${root}/node_modules/playwright-core/index.mjs`).href);
const manifest=JSON.parse(await readFile(`${build}/manifest.json`,'utf8'));
const entry=manifest['resources/react/main.tsx'];
const credential=config.credential;
const headers={'Authorization':`Bearer ${credential.token}`,'X-Voyage-Grant':credential.grant_id,'X-Voyage-Vessel':credential.vessel_id,'Content-Type':'application/json'};
const bootstrap={tenantId:'goal-process-fixture',vessels:[{id:credential.vessel_id,vessel_id:credential.vessel_id,name:'Goal fixture Vessel'}],pairings:[],ticketUrl:'/console/ticket',connectionsUrl:'/connections',logoutUrl:'/console/logout'};
const html=`<!doctype html><html><head><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="csrf-token" content="fixture">${entry.css.map(css=>`<link rel="stylesheet" href="/build/${css}">`).join('')}</head><body><div id="helm-react" data-bootstrap='${JSON.stringify(bootstrap)}'></div><script type="module" src="/build/${entry.file}"></script></body></html>`;
let origin;
const server=createServer({key:await readFile(config.key),cert:await readFile(config.cert)},async(req,res)=>{
    try {
        if(req.url==='/console/ticket'){
            const response=await fetch(`${config.gateway}/v1/vessel/browser-credentials`,{method:'POST',headers,body:JSON.stringify({origin})});
            assert.equal(response.status,200,'real browser credential refused');
            const ticket=await response.json();res.setHeader('Content-Type','application/json');
            res.end(JSON.stringify({...ticket,url:'wss://fixture.invalid/v1/vessel/browser-socket'}));return;
        }
        if(req.url==='/'||req.url.startsWith('/voyages/')){res.setHeader('Content-Type','text/html');res.end(html);return;}
        const path=resolve(build,`.${new URL(req.url,'https://fixture').pathname.replace(/^\/build/,'')}`);
        if(!path.startsWith(build+'/')){res.writeHead(404).end();return;}
        res.setHeader('Content-Type',extname(path)==='.css'?'text/css':'text/javascript');res.end(await readFile(path));
    }catch(error){console.error(error.message);res.writeHead(500).end();}
});
const sockets=new Set();
server.on('connection',socket=>{sockets.add(socket);socket.on('close',()=>sockets.delete(socket));});
server.on('upgrade',(req,socket,head)=>{
    assert.equal(req.url,'/v1/vessel/browser-socket');
    const destination=new URL(config.gateway);
    const upstream=connect(Number(destination.port),'127.0.0.1',()=>{
        upstream.write(`${req.method} ${req.url} HTTP/1.1\r\n${Object.entries(req.headers).map(([key,value])=>`${key}: ${key==='host'?destination.host:value}`).join('\r\n')}\r\n\r\n`);
        if(head.length)upstream.write(head);socket.pipe(upstream);upstream.pipe(socket);
    });
    socket.on('error',()=>upstream.destroy());upstream.on('error',()=>socket.destroy());
    socket.on('close',()=>upstream.destroy());upstream.on('close',()=>socket.destroy());
});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
origin=`https://127.0.0.1:${server.address().port}`;
const browser=await chromium.launch({executablePath:process.env.CHROMIUM_PATH||'/usr/bin/chromium',headless:true});
const errors=[],frames=[];
let page;
const waitForFile=async path=>{const end=Date.now()+45000;while(Date.now()<end){try{return await readFile(path,'utf8');}catch{}await new Promise(resolve=>setTimeout(resolve,100));}throw Error('Other client did not finish');};
try {
    const context=await browser.newContext({viewport:{width:1440,height:900},ignoreHTTPSErrors:true,reducedMotion:'reduce'});
    page=await context.newPage();page.on('pageerror',error=>errors.push(error.message));
    page.on('websocket',socket=>{socket.on('framereceived',event=>frames.push({received:JSON.parse(event.payload)}));socket.on('framesent',event=>{const value=JSON.parse(event.payload);if(value.type!=='authenticate')frames.push({sent:value});});});
    await page.route('**/*',route=>route.request().url().startsWith(origin+'/')?route.continue():route.abort());
    await page.addInitScript(destination=>{
        const NativeSocket=window.WebSocket;
        window.WebSocket=class extends NativeSocket{
            constructor(url,protocols){super(new URL(url).hostname==='fixture.invalid'?destination:url,protocols);}
        };
    },origin.replace('https:','wss:')+'/v1/vessel/browser-socket');
    await page.goto(origin);
    await page.locator('.voyage-card').click();
    await page.getByRole('button',{name:'Set goal',exact:true}).click();
    let dialog=page.getByRole('dialog',{name:'Set goal',exact:true});
    await dialog.getByRole('textbox',{name:'Objective',exact:true}).fill('goal-case-budget Web and TUI shared objective <script>plain</script>');
    assert.equal(await dialog.getByRole('checkbox').isChecked(),false);
    await dialog.getByRole('button',{name:'Save paused goal',exact:true}).click();
    await page.getByRole('button',{name:'Goal · Paused',exact:true}).click();
    dialog=page.getByRole('dialog',{name:'Voyage goal',exact:true});
    assert.equal(await dialog.locator('script').count(),0);
    await dialog.getByRole('button',{name:'Edit objective or limits',exact:true}).click();
    dialog=page.getByRole('dialog',{name:'Edit goal',exact:true});
    await dialog.getByRole('textbox',{name:'Objective',exact:true}).fill('A stale Web edit must not win');
    await writeFile(`${config.evidence}/goal-web-review-ready`,'ready');
    await waitForFile(`${config.evidence}/goal-web-other-client-done`);
    await dialog.getByRole('button',{name:'Save and pause',exact:true}).click();
    await dialog.getByRole('alert').waitFor();
    assert.match(await dialog.getByRole('alert').innerText(),/changed|review|revision conflict/i);
    await page.keyboard.press('Escape');
    await page.reload();
    await page.locator('.voyage-card').click();
    await page.getByRole('button',{name:'Goal · Paused',exact:true}).click();
    dialog=page.getByRole('dialog',{name:'Voyage goal',exact:true});
    await dialog.getByText('0 / 9,000',{exact:true}).waitFor();
    await dialog.evaluate(element=>Promise.all(element.getAnimations({subtree:true}).map(animation=>animation.finished)));
    await page.screenshot({path:`${config.evidence}/goal-web-canonical.png`});
    await dialog.getByRole('button',{name:'Clear goal',exact:true}).click();
    dialog=page.getByRole('dialog',{name:'Clear goal',exact:true});
    assert.equal(await dialog.getByRole('button',{name:'Clear goal',exact:true}).isDisabled(),true);
    await dialog.getByRole('checkbox').check();
    await dialog.getByRole('button',{name:'Clear goal',exact:true}).click();
    await page.getByRole('button',{name:'Set goal',exact:true}).waitFor();
    assert.deepEqual(errors,[]);
    await writeFile(`${config.evidence}/goal-web-result.json`,JSON.stringify({chromium:browser.version(),asset:entry.file,realProtocol:true,realVoyage:true,fixtureTls:true,checks:['create-paused','untrusted-text','concurrent-TUI-stale-review','reconnect-canonical-limits','confirmed-clear'],errors},null,2));
}catch(error){
    if(page){await page.screenshot({path:`${config.evidence}/goal-web-failure.png`});await writeFile(`${config.evidence}/goal-web-failure.txt`,await page.locator('body').innerText());}
    throw error;
}finally{
    await writeFile(`${config.evidence}/goal-web-frames.json`,JSON.stringify(frames,null,2));
    await browser.close();for(const socket of sockets)socket.destroy();await new Promise(resolve=>server.close(resolve));
}
