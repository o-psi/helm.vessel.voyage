import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import React from 'react';


test('React journey connects, preserves drafts on navigation and submits once',async()=>{
 const dom=new JSDOM('<meta name="csrf-token" content="fixture"><div id="mount"></div>',{url:'https://helm.test/react',pretendToBeVisual:true});
 dom.window.HTMLDialogElement.prototype.showModal=function(){this.open=true;};dom.window.HTMLDialogElement.prototype.close=function(){this.open=false;};
 const saved:any={};for(const key of ['window','document','location','localStorage','Event','CustomEvent']){saved[key]=(globalThis as any)[key];(globalThis as any)[key]=(dom.window as any)[key];}
 Object.defineProperty(dom.window,'matchMedia',{value:()=>({matches:false,addEventListener(){},removeEventListener(){}})});
 (globalThis as any).IS_REACT_ACT_ENVIRONMENT=true;
 const oldFetch=globalThis.fetch,oldSocket=globalThis.WebSocket;let mutations=0;const commands:any[]=[];
 class Socket extends dom.window.EventTarget {
  readyState=0;protocol='voyage.vessel.v1';
  constructor(){super();queueMicrotask(()=>{this.readyState=1;this.dispatchEvent(new dom.window.Event('open'));});}
  send(text:string){const frame=JSON.parse(text);if(frame.type==='authenticate'){queueMicrotask(()=>this.dispatchEvent(new dom.window.MessageEvent('message',{data:JSON.stringify({type:'hello',protocol:1,vessel_id:'v',socket_id:'socket'})})));return;}
   if(frame.type==='subscribe'||frame.type==='unsubscribe')return;
   const c=frame.request.command;commands.push(c);let result:any;
   if(c.op==='capabilities')result={scope:'owner',vessel_id:'v'};
   else if(c.op==='catalogue')result=['a','b'].map(id=>({session_id:id,incarnation:'i',name:`Voyage ${id}`,state:'live',catalogue:{summary:{run_state:'idle'}}}));
   else if(c.op==='snapshot')result={session_id:c.session_id,incarnation:'i',result:{session_id:c.session_id,name:`Voyage ${c.session_id}`,revision:1,observation_cursor:5,messages:[{role:'assistant',content:'**Hello**',message_index:0}],run:{state:'idle'}}};
   else if(c.op==='decisions')result={session_id:c.session_id,incarnation:'i',result:[]};
   else if(c.op==='submit'){mutations++;result={session_id:c.session_id,incarnation:'i',result:{command_id:c.command_id,status:'accepted'}};}
   else throw new Error(`Unexpected operation ${c.op}`);
   queueMicrotask(()=>this.dispatchEvent(new dom.window.MessageEvent('message',{data:JSON.stringify({type:'reply',request_id:frame.request_id,response:{protocol:1,outcome_unknown:false,result}})})));
  }
  close(){this.readyState=3;this.dispatchEvent(new dom.window.Event('close'));}
 }
 globalThis.WebSocket=Socket as any;globalThis.fetch=async()=>({ok:true,json:async()=>({url:'wss://vessel.example/v1/vessel/browser-socket',vessel_id:'v',token:'a'.repeat(64),expires_at_ms:Date.now()+120000})}) as any;
 const {createRoot}=await import('react-dom/client');
 const {App}=await import('../resources/react/App.tsx');const root=createRoot(dom.window.document.querySelector('#mount')!);
 const settle=()=>new Promise(resolve=>setTimeout(resolve,10));
 try{
  await React.act(async()=>{root.render(React.createElement(App,{bootstrap:{tenantId:'t',vessels:[{id:'c',vessel_id:'v',name:'Vessel'}],ticketUrl:'/console/ticket',legacyUrl:'/',connectionsUrl:'/connections',logoutUrl:'/console/logout'}}));await settle();});
  assert.equal(dom.window.document.querySelectorAll('.voyage-card').length,2);
  await React.act(async()=>{dom.window.document.querySelector<HTMLButtonElement>('.voyage-card')!.click();await settle();});
  const active=()=>dom.window.document.querySelector<HTMLElement>('.conversation:not([hidden])')!;
  assert.match(active().textContent!,/Hello/);assert.equal(active().querySelector('.prose strong')?.textContent,'Hello');
  const field=active().querySelector<HTMLTextAreaElement>('textarea')!;
  await React.act(async()=>{const setter=Object.getOwnPropertyDescriptor(dom.window.HTMLTextAreaElement.prototype,'value')!.set!;setter.call(field,'Retained draft');field.dispatchEvent(new dom.window.Event('input',{bubbles:true}));});
  await React.act(async()=>{dom.window.document.querySelectorAll<HTMLButtonElement>('.voyage-card')[1].click();await settle();});
  await React.act(async()=>{dom.window.document.querySelectorAll<HTMLButtonElement>('.voyage-card')[0].click();await settle();});
  assert.equal(active().querySelector('textarea'),field);assert.equal(field.value,'Retained draft');
  await React.act(async()=>{active().querySelector('form')!.dispatchEvent(new dom.window.Event('submit',{bubbles:true,cancelable:true}));await settle();});
  assert.equal(mutations,1);assert.equal(commands.find(c=>c.op==='submit').prompt,'Retained draft');assert.equal(field.value,'');
 }finally{await React.act(async()=>root.unmount());globalThis.fetch=oldFetch;globalThis.WebSocket=oldSocket;Object.assign(globalThis,saved);dom.window.close();}
});
