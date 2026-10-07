import {randomUUID} from 'node:crypto';
// Disposable visual inspection; no website state or execution bridge crosses in.
export const MAX_HTML=128*1024;
export function validatePreview(a){
  if(typeof a?.html!=='string'||!a.html.trim()||Buffer.byteLength(a.html)>MAX_HTML||!Number.isSafeInteger(a.width)||a.width<240||a.width>1600||!['dark','light'].includes(a.appearance))throw Error('invalid HTML preview');
}
export async function htmlPreview(worker,a){
  validatePreview(a);
  const context=await worker.task.newContext({viewport:{width:a.width,height:640},acceptDownloads:false,serviceWorkers:'block',colorScheme:a.appearance});
  worker.previewContext=context;
  const deadline=setTimeout(()=>{void context.close().catch(()=>{});},8000);
  try{
    context.setDefaultTimeout(3000);context.setDefaultNavigationTimeout(3000);
    await context.route('**/*',route=>route.abort());
    await context.routeWebSocket('**/*',socket=>socket.close());
    await context.addInitScript(()=>{for(const key of ['RTCPeerConnection','webkitRTCPeerConnection'])Object.defineProperty(globalThis,key,{value:undefined,configurable:false});});
    const page=await context.newPage(),consoleMessages=[];let diagnosticBytes=0;
    const log=(level,text)=>{text=String(text).slice(0,2048);const size=Buffer.byteLength(text);if(consoleMessages.length<32&&diagnosticBytes+size<=16384){diagnosticBytes+=size;consoleMessages.push({level,text});}};
    page.on('console',message=>log(['log','info','warning','error'].includes(message.type())?message.type():'log',message.text()));
    page.on('pageerror',error=>log('error',error.message));page.on('dialog',dialog=>dialog.dismiss());
    // Authored HTML runs only in an opaque sandbox. Prefix CSP prevents even
    // subframe/navigation resources; route guards are independent defense.
    const csp="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; worker-src 'none'";
    const dark=a.appearance==='dark';
    const style=`<style>:root{--background:${dark?'#171717':'#ffffff'};--foreground:${dark?'#fafafa':'#171717'};--muted:${dark?'#262626':'#f5f5f5'};--muted-foreground:${dark?'#a3a3a3':'#737373'};--border:${dark?'#404040':'#e5e5e5'};--primary:${dark?'#fafafa':'#171717'};--primary-foreground:${dark?'#171717':'#fafafa'};--card:var(--background);--card-foreground:var(--foreground);color-scheme:${a.appearance}}body{margin:0;background:var(--background);color:var(--foreground)}</style>`;
    const ready=randomUUID();
    const prefix=`<meta http-equiv="Content-Security-Policy" content="${csp}">${style}<script>addEventListener('DOMContentLoaded',()=>document.documentElement.setAttribute('data-helm-preview-loaded',${JSON.stringify(ready)}),{once:true});</script>`;
    // Literal srcdoc encoding is performed by Playwright's attribute setter in
    // a trusted blank parent, never by HTML-string interpolation of the model.
    await page.setContent('<meta http-equiv="Content-Security-Policy" content="frame-src &apos;none&apos;"><style>body{margin:0}</style>',{timeout:3000});
    await page.evaluate(html=>{const frame=document.createElement('iframe');frame.setAttribute('sandbox','allow-scripts');frame.style.cssText='border:0;width:100%;height:640px';frame.title='Preview';frame.srcdoc=html;document.body.append(frame);},prefix+a.html);
    const document=page.frameLocator('iframe').locator(`html[data-helm-preview-loaded="${ready}"]`);
    await document.waitFor({state:'attached',timeout:3000});
    const contentHeight=await document.evaluate(()=>Math.min(100000,Math.ceil(Math.max(window.document.body?.scrollHeight||0,window.document.body?.getBoundingClientRect().height||0))));
    const capturedHeight=Math.max(180,Math.min(640,contentHeight));
    await page.setViewportSize({width:a.width,height:capturedHeight});
    await page.locator('iframe').evaluate((frame,height)=>frame.style.height=height+'px',capturedHeight);
    const bytes=await page.screenshot({type:'png',timeout:3000});
    if(bytes.length>2*1024*1024)throw Error('preview screenshot bound');
    return {width:a.width,contentHeight,capturedHeight,consoleMessages,mime_type:'image/png',data_base64:bytes.toString('base64')};
  }finally{
    // Keep the obligation reachable until Chromium confirms the context closed.
    clearTimeout(deadline);await context.close();if(worker.previewContext===context)worker.previewContext=null;
  }
}
