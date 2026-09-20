import test from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import {JSDOM} from 'jsdom';
import {threadRows,ToolGroup} from '../resources/react/ToolGroup.tsx';
const messages=Array.from({length:5},(_,i)=>[{role:'assistant',message_index:i*2,content:'',tool_calls:[{id:`call-${i}`,function:{name:'shell',arguments:`command-${i}`}}]},{role:'tool',message_index:i*2+1,tool_call_id:`call-${i}`,content:`result-${i}`}]).flat();
test('tool calls and results become one group with no duplicate result messages',()=>{
 const rows=threadRows(messages);assert.equal(rows.length,1);assert.equal(rows[0].entries?.length,5);assert.equal(rows[0].entries?.[0].result.content,'result-0');
 const split=threadRows([...messages,{role:'assistant',message_index:10,content:'Explanation',tool_calls:[{id:'final'}]},{role:'tool',message_index:11,tool_call_id:'final',content:'Done'}]);assert.equal(split.length,3);assert.equal(split[1].message.content,'Explanation');assert.equal(split[2].entries?.length,1);
});
test('older actions hide by default; results mount only on expansion and disclosure survives rerender',async()=>{
 const dom=new JSDOM('<div id="root"></div>');const previous={window:globalThis.window,document:globalThis.document};Object.assign(globalThis,{window:dom.window,document:dom.window.document,IS_REACT_ACT_ENVIRONMENT:true});
 const {createRoot}=await import('react-dom/client');const root=createRoot(document.querySelector('#root')!);const entries=threadRows(messages)[0].entries!;
 const render=()=>React.createElement(ToolGroup,{entries,running:false,messageStart:0,decisions:false,renderMessage:(message:any)=>React.createElement('pre',null,message.content)});
 try{
  await React.act(async()=>root.render(render()));assert.equal(document.querySelectorAll('.tool-group > div[hidden]').length,2);assert.doesNotMatch(document.body.textContent!,/result-4|command-4/);
  const detail=document.querySelector<HTMLDetailsElement>('[data-tool-id="call-4"]')!;
  await React.act(async()=>{detail.open=true;detail.dispatchEvent(new dom.window.Event('toggle'));});assert.match(detail.textContent!,/result-4/);
  await React.act(async()=>root.render(render()));assert.equal(detail.open,true);
  await React.act(async()=>{document.querySelector<HTMLButtonElement>('.tool-group-heading button')!.click();});assert.equal(document.querySelectorAll('.tool-group > div[hidden]').length,0);
  await React.act(async()=>{document.querySelector<HTMLButtonElement>('.tool-group-heading button')!.click();});assert.equal(document.querySelectorAll('.tool-group > div[hidden]').length,2);
  await React.act(async()=>{detail.open=false;detail.dispatchEvent(new dom.window.Event('toggle'));});assert.doesNotMatch(detail.textContent!,/result-4/);
 }finally{await React.act(async()=>root.unmount());Object.assign(globalThis,previous);dom.window.close();}
});
