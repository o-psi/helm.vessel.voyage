import test from 'node:test';
import assert from 'node:assert/strict';
import React from 'react';
import {createRoot} from 'react-dom/client';
import {JSDOM} from 'jsdom';
import {Conversation} from '../resources/react/App.tsx';

test('transcript preloads at the top edge, preserves the anchor and avoids duplicate reads', async () => {
    const dom = new JSDOM('<div id="mount"></div>', {pretendToBeVisual:true});
    const saved = {window:globalThis.window, document:globalThis.document, requestAnimationFrame:globalThis.requestAnimationFrame};
    Object.assign(globalThis, {window:dom.window, document:dom.window.document, requestAnimationFrame:(fn:FrameRequestCallback)=>fn(0), IS_REACT_ACT_ENVIRONMENT:true});
    const root = createRoot(dom.window.document.querySelector('#mount')!);
    const tab:any = {key:'t', title:'Voyage', snapshot:{message_offset:100, messages:[]}, decisions:[], pictures:[]};
    let reads = 0;
    const workspace:any = {actionable:()=>false,permitted:()=>false,earlier:async()=>{reads++;tab.snapshot={...tab.snapshot,message_offset:50,messages:tab.snapshot.messages};}};
    try {
        await React.act(async()=>root.render(React.createElement(Conversation,{tab,workspace,active:false,onSettings:()=>{}})));
        const el = dom.window.document.querySelector<HTMLElement>('.transcript')!;
        let height = 2000;
        Object.defineProperties(el,{clientHeight:{get:()=>800},scrollHeight:{get:()=>height}});
        el.scrollTop=1200;
        await React.act(async()=>root.render(React.createElement(Conversation,{tab,workspace,active:true,onSettings:()=>{}})));
        await React.act(async()=>el.dispatchEvent(new dom.window.Event('scroll',{bubbles:true})));
        assert.equal(reads,0);
        el.scrollTop=100;
        await React.act(async()=>{el.dispatchEvent(new dom.window.Event('scroll',{bubbles:true}));height=2600;await new Promise(resolve=>setTimeout(resolve,0));});
        assert.equal(reads,1);
        assert.equal(el.scrollTop,700);
        assert.doesNotMatch(dom.window.document.body.textContent!,/Load earlier messages/);
        await React.act(async()=>el.dispatchEvent(new dom.window.Event('scroll',{bubbles:true})));
        assert.equal(reads,1);
    } finally {await React.act(async()=>root.unmount());Object.assign(globalThis,saved);dom.window.close();}
});


test('OS picture selection displays a composer error instead of silently disappearing', async () => {
    const dom = new JSDOM('<div id="mount"></div>', {pretendToBeVisual:true});
    const saved = {window:globalThis.window, document:globalThis.document, requestAnimationFrame:globalThis.requestAnimationFrame};
    Object.assign(globalThis, {window:dom.window, document:dom.window.document, requestAnimationFrame:(fn:FrameRequestCallback)=>fn(0), IS_REACT_ACT_ENVIRONMENT:true});
    const root = createRoot(dom.window.document.querySelector('#mount')!);
    const tab:any = {key:'t', title:'Voyage', snapshot:{messages:[]}, decisions:[], pictures:[], draft:'', notice:'', busy:false};
    const workspace:any = {actionable:()=>false, permitted:()=>false, attach:async(_key:string,files:File[])=>{
        assert.equal(files[0].name,'unsupported.svg');tab.notice='Use PNG, JPEG or WebP; at most four pictures and 2 MiB total.';return false;
    }};
    try {
        await React.act(async()=>root.render(React.createElement(Conversation,{tab,workspace,active:true,onSettings:()=>{}})));
        const input = dom.window.document.querySelector<HTMLInputElement>('input[type=file]')!;
        Object.defineProperty(input,'files',{configurable:true,value:[new dom.window.File(['svg'],'unsupported.svg',{type:'image/svg+xml'})]});
        await React.act(async()=>{input.dispatchEvent(new dom.window.Event('change',{bubbles:true}));await Promise.resolve();});
        await React.act(async()=>root.render(React.createElement(Conversation,{tab,workspace,active:true,onSettings:()=>{}})));
        assert.match(dom.window.document.querySelector('.composer')!.textContent!,/Use PNG, JPEG or WebP/);
    } finally {await React.act(async()=>root.unmount());Object.assign(globalThis,saved);dom.window.close();}
});


test('OS picture selection shows a named preview in the composer', async () => {
    const dom = new JSDOM('<div id="mount"></div>', {pretendToBeVisual:true});
    const saved = {window:globalThis.window, document:globalThis.document, requestAnimationFrame:globalThis.requestAnimationFrame};
    Object.assign(globalThis, {window:dom.window, document:dom.window.document, requestAnimationFrame:(fn:FrameRequestCallback)=>fn(0), IS_REACT_ACT_ENVIRONMENT:true});
    const root = createRoot(dom.window.document.querySelector('#mount')!);
    const tab:any = {key:'t', title:'Voyage', snapshot:{messages:[]}, decisions:[], pictures:[], draft:'', notice:'', busy:false};
    const workspace:any = {actionable:()=>true,permitted:()=>true,attach:async(_key:string,files:File[])=>{
        tab.pictures.push({id:'p',name:files[0].name,url:'blob:preview'});return true;
    }};
    try {
        await React.act(async()=>root.render(React.createElement(Conversation,{tab,workspace,active:true,onSettings:()=>{}})));
        const input=dom.window.document.querySelector<HTMLInputElement>('input[type=file]')!;
        Object.defineProperty(input,'files',{configurable:true,value:[new dom.window.File(['png'],'photo.png',{type:'image/png'})]});
        await React.act(async()=>{input.dispatchEvent(new dom.window.Event('change',{bubbles:true}));await Promise.resolve();});
        await React.act(async()=>root.render(React.createElement(Conversation,{tab,workspace,active:true,onSettings:()=>{}})));
        assert.equal(dom.window.document.querySelector('.composer .pictures img')?.getAttribute('alt'),'photo.png');
        assert.match(dom.window.document.querySelector('.composer')!.textContent!,/photo.png/);
        assert.equal(dom.window.document.querySelector<HTMLButtonElement>('.composer .send-button')?.disabled,false);
    } finally {await React.act(async()=>root.unmount());Object.assign(globalThis,saved);dom.window.close();}
});
