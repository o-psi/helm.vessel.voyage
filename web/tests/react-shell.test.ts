import test from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import {renderToStaticMarkup} from 'react-dom/server';
import {JSDOM} from 'jsdom';
import {App} from '../resources/react/App.tsx';

test('React shell renders safely without Livewire and exposes migration boundaries', () => {
    const dom = new JSDOM('<meta name="csrf-token" content="fixture-token">');
    const previous = globalThis.document;
    globalThis.document = dom.window.document;
    try {
        const html = renderToStaticMarkup(React.createElement(App, {bootstrap: {tenantId: 'tenant', vessels: [{id: 'v', name: '<img src=x onerror=alert(1)>', vessel_id: 'owner'}], ticketUrl: '/console/ticket', legacyUrl: '/', connectionsUrl: '/connections', logoutUrl: '/console/logout'}}));
        const output = new JSDOM(html).window.document;
        assert.equal(output.querySelectorAll('img,script').length, 0);
        assert.match(output.body.textContent!, /<img src=x onerror=alert\(1\)>/);
        assert.match(output.body.textContent!, /React preview/);
        assert.match(output.body.textContent!, /Compare with existing console/);
        assert.equal(output.querySelector('.tabs'), null);
        assert.equal(output.querySelector('.welcome'), null);
        assert.equal(output.querySelector('textarea')?.getAttribute('placeholder'), 'Ask anything…');
        assert.ok(output.querySelector('[aria-label="New voyage"]'));
        assert.ok(output.querySelector('[aria-label="Open voyage navigation"]'));
        assert.equal(output.querySelectorAll('.appearance button').length, 3);
        assert.equal(output.querySelector('form')?.getAttribute('method'), 'post');
        assert.equal(output.querySelector<HTMLInputElement>('[name="_token"]')?.value, 'fixture-token');
        assert.equal(output.querySelector('nav')?.getAttribute('aria-label'), 'Voyages');
        assert.equal(output.querySelectorAll('[wire\\:id]').length, 0);
    } finally { globalThis.document = previous; dom.window.close(); }
});

test('preview preserves console responsive widths, theme, card and composer styling', async () => {
    const {readFileSync} = await import('node:fs');
    const css = readFileSync(new URL('../resources/react/style.css', import.meta.url), 'utf8');
    assert.match(css, /grid-template-columns:256px minmax\(0,1fr\)/);
    assert.match(css, /\.dark\{color-scheme:dark/);
    assert.match(css, /max-width:80rem/);
    assert.match(css, /max-width:96rem/);
    assert.match(css, /border-radius:24px/);
    assert.match(css, /prefers-reduced-motion:reduce/);
    assert.match(css, /\.sidebar\.mobile-open/);
    assert.match(css, /\.conversation\[hidden\]\{display:none\}/);
});

test('React approval UI distinguishes root consent from generic approval', async()=>{
 const {Decisions}=await import('../resources/react/Decisions.tsx');
 const tab:any={key:'a',vessel:'host',incarnation:'inc',snapshot:{run:{run_id:'run'}},decisions:[{decision_id:'root',incarnation:'inc',run_id:'run',expires_at_ms:Date.now()+10000,request:{kind:'root_grant',root_grant:{path:'/workspace',permission:'write',lifetime:'current_run',reason:'Edit project'}}}]};
 const html=renderToStaticMarkup(React.createElement(Decisions,{tab,workspace:{actionable:()=>true,permitted:()=>true} as any}));
 assert.match(html,/Grant access for this run/);assert.match(html,/Not inherited by children/);assert.doesNotMatch(html,/>Approve</);
 tab.decisions[0].request.root_grant.path='/workspace\nspoof';
 const invalid=renderToStaticMarkup(React.createElement(Decisions,{tab,workspace:{actionable:()=>true,permitted:()=>true} as any}));
 assert.match(invalid,/Unsupported filesystem request/);assert.doesNotMatch(invalid,/Grant access for this run/);
});

test('React enrollment island mounts and scrubs private state on unmount',async()=>{
 const {Enrollment}=await import('../resources/react/Enrollment.tsx');
 const {createRoot}=await import('react-dom/client');
 const {act}=React;
 const dom=new JSDOM('<div id="mount"></div>',{url:'https://helm.test'});
 const saved={window:globalThis.window,document:globalThis.document,localStorage:globalThis.localStorage};
 Object.assign(globalThis,{window:dom.window,document:dom.window.document,localStorage:dom.window.localStorage,IS_REACT_ACT_ENVIRONMENT:true});
 const root=createRoot(dom.window.document.querySelector('#mount')!);
 const connection:any={id:'c',vessel_id:'v',client:{socket:new dom.window.EventTarget(),async exchange(){return {protocol:1,outcome_unknown:false,result:{accounts:[],connections:[]}};}}};
 try{
  await act(async()=>root.render(React.createElement(Enrollment,{connection,workspace:'/work',tenant:'t',onRefreshed:()=>{}})));
  await act(async()=>{dom.window.document.querySelector<HTMLButtonElement>('[data-open]')!.click();});
  assert.match(dom.window.document.body.textContent!,/No ChatGPT connection is available/);
  await act(async()=>root.unmount());assert.equal(dom.window.document.querySelector('#enrollment-code'),null);
 }finally{Object.assign(globalThis,saved);dom.window.close();}
});

test('React voyage action island opens audited details and cleans up',async()=>{
 const {VoyageActions}=await import('../resources/react/VoyageActions.tsx');const {createRoot}=await import('react-dom/client');const {act}=React;
 const dom=new JSDOM('<div id="mount"></div>',{url:'https://helm.test'});
 dom.window.HTMLDialogElement.prototype.showModal=function(){this.open=true;};dom.window.HTMLDialogElement.prototype.close=function(){this.open=false;};
 const saved={window:globalThis.window,document:globalThis.document};Object.assign(globalThis,{window:dom.window,document:dom.window.document,IS_REACT_ACT_ENVIRONMENT:true});
 const commands:string[]=[];const connection:any={id:'c',vessel_id:'v',name:'Vessel',journal:{entries:()=>[]},client:{async exchange({command}:any){commands.push(command.op);const result=command.op==='capabilities'?{scope:'owner',vessel_id:'v'}:command.op==='inspect'?{session_id:'s',incarnation:'i',state:'live'}:{session_id:'s',incarnation:'i',result:{session_id:'s',revision:1,run:{state:'idle'}}};return {protocol:1,outcome_unknown:false,result};}}};
 const root=createRoot(dom.window.document.querySelector('#mount')!);
 try{await act(async()=>root.render(React.createElement(VoyageActions,{connection,voyage:{session_id:'s',name:'Voyage'},onChanged:()=>{}})));
 await act(async()=>{[...dom.window.document.querySelectorAll<HTMLButtonElement>('[data-actions] button')].find(button=>button.textContent==='Details')!.click();});
 assert.equal(dom.window.document.querySelector('dialog')?.open,true);assert.match(dom.window.document.querySelector('#sidebar-action-details')!.textContent!,/"revision": 1/);assert.deepEqual(commands,['capabilities','inspect','snapshot']);
 await act(async()=>root.unmount());assert.equal(dom.window.document.querySelector('dialog'),null);
 }finally{Object.assign(globalThis,saved);dom.window.close();}
});
