import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import React, {act} from 'react';
import {createRoot} from 'react-dom/client';
import {JSDOM} from 'jsdom';
import {HostBrowser} from '../resources/react/HostBrowser.tsx';
import type {Tab} from '../resources/react/workspace.ts';

test('mobile viewer traps focus, Escape restores action and conversation DOM survives', async () => {
    const dom = new JSDOM('<main id="app" class="voyage-workspace"></main>');
    const saved = ['window','document','IS_REACT_ACT_ENVIRONMENT'].map(name => [name,Object.getOwnPropertyDescriptor(globalThis,name)] as const);
    Object.assign(globalThis,{window:dom.window,document:dom.window.document,IS_REACT_ACT_ENVIRONMENT:true});
    dom.window.HTMLElement.prototype.getClientRects=function(){return [{width:40,height:40}] as any;};
    Object.defineProperty(dom.window,'matchMedia',{value:()=>({matches:true})});
    const root = createRoot(dom.window.document.getElementById('app')!);
    const tab = {key:'one',session:'one',title:'Research trip',stale:true,snapshot:{messages:[{role:'tool',name:'host_browser'}]}} as Tab;
    try {
        await act(async()=>root.render(React.createElement(React.Fragment,null,
            React.createElement(HostBrowser,{tab,client:null}),
            React.createElement('section',{className:'conversation'},React.createElement('div',{className:'transcript'}),React.createElement('textarea',{defaultValue:'Unsent draft'})))));
        const draft = dom.window.document.querySelector('textarea')!, transcript = dom.window.document.querySelector('.transcript')!;
        transcript.scrollTop = 321;
        const action = dom.window.document.querySelector('button')!;
        assert.equal(action.textContent,'Browser');
        assert.match(dom.window.document.getElementById(action.getAttribute('aria-describedby')!)!.textContent!,/activity/);
        await act(async()=>action.click());
        const panel = dom.window.document.querySelector('aside')!;
        assert.equal(dom.window.document.getElementById(panel.getAttribute('aria-labelledby')!)!.textContent,'Browser');
        assert.match(panel.textContent!,/Research trip/);
        assert.equal(dom.window.document.activeElement,panel);
        dom.window.document.dispatchEvent(new dom.window.KeyboardEvent('keydown',{key:'Tab',bubbles:true,cancelable:true}));
        assert.equal(dom.window.document.activeElement?.getAttribute('aria-label'),'Close browser viewer');
        await act(async()=>dom.window.document.dispatchEvent(new dom.window.KeyboardEvent('keydown',{key:'Escape',bubbles:true})));
        assert.equal(dom.window.document.querySelector('aside'),null);
        assert.equal(dom.window.document.activeElement,action);
        assert.equal(dom.window.document.querySelector('textarea'),draft);
        assert.equal(draft.value,'Unsent draft');
        assert.equal(transcript.scrollTop,321);
    } finally {
        await act(async()=>root.unmount());
        for (const [name,descriptor] of saved) { if(descriptor) Object.defineProperty(globalThis,name,descriptor); else delete (globalThis as any)[name]; }
        dom.window.close();
    }
});

test('desktop split and mobile overlay CSS contracts for both shells', () => {
    // JSDOM does not implement viewport layout: actual screenshots are a separate integration check.
    const react = readFileSync(new URL('../resources/react/style.css',import.meta.url),'utf8');
    const livewire = readFileSync(new URL('../resources/css/console.css',import.meta.url),'utf8');
    assert.match(react,/grid-template-columns:minmax\(300px,32%\) minmax\(0,1fr\)/);
    assert.match(react,/\.task-browser-panel\.expanded/);
    assert.match(react,/@media\(max-width:1000px\)[\s\S]*\.task-browser-panel\{position:fixed;inset:0/);
    assert.match(livewire,/grid-template-columns: minmax\(300px, 32%\) minmax\(0, 1fr\)/);
    assert.match(livewire,/\.browser-workspace\.browser-expanded/);
    assert.match(livewire,/@media \(max-width: 1000px\)[\s\S]*position: fixed; inset: 0/);
});
