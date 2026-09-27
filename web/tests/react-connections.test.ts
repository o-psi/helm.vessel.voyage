import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import React from 'react';

test('Vessel manager separates overview, setup and destructive confirmation', async () => {
    const dom = new JSDOM('<div id="mount"></div>', {url: 'https://helm.test/', pretendToBeVisual: true});
    dom.window.HTMLDialogElement.prototype.showModal = function () { this.open = true; };
    dom.window.HTMLDialogElement.prototype.close = function () { this.open = false; };
    const saved: Record<string, unknown> = {};
    for (const key of ['window', 'document', 'location', 'Event']) {
        saved[key] = (globalThis as any)[key];
        (globalThis as any)[key] = (dom.window as any)[key];
    }
    (globalThis as any).IS_REACT_ACT_ENVIRONMENT = true;
    const oldFetch = globalThis.fetch;
    let requests = 0;
    globalThis.fetch = async () => { requests++; throw Error('network lost'); };
    const {createRoot} = await import('react-dom/client');
    const {Connections} = await import('../resources/react/Connections.tsx');
    const root = createRoot(dom.window.document.querySelector('#mount')!);
    const query = (selector: string) => dom.window.document.querySelector(selector);
    const click = async (label: string) => React.act(async () => {
        const button = [...dom.window.document.querySelectorAll('button')].find(node => node.textContent?.includes(label));
        assert.ok(button, `missing ${label}`);
        button.click();
    });
    const enter = async (selector: string, value: string) => React.act(async () => {
        const input = query(selector) as HTMLInputElement | HTMLTextAreaElement;
        const setter = Object.getOwnPropertyDescriptor(Object.getPrototypeOf(input), 'value')!.set!;
        setter.call(input, value);
        input.dispatchEvent(new dom.window.Event('input', {bubbles: true}));
    });
    try {
        await React.act(async () => root.render(React.createElement(Connections, {
            bootstrap: {vessels: [{id: 'c', name: 'Workstation', vessel_id: 'v', endpoint: 'https://vessel.example'}], pairings: [{id: 'p', name: 'Laptop'}]},
            states: {c: {connected: true, status: 'Connected'}}, onClose: () => {},
        })));
        assert.match(query('.connections-card')!.textContent!, /Connected/);
        assert.match(query('.connections-card')!.textContent!, /vessel.example/);
        assert.match(query('.connections-pending')!.textContent!, /Laptop/);
        await click('View details');
        assert.equal(query('.connections-confirm'), null);
        await click('Remove connection');
        assert.ok(query('.connections-confirm'));
        assert.equal(requests, 0);
        await click('Keep connection');
        assert.equal(query('.connections-confirm'), null);
        await click('Back to Vessels');
        await click('Add Vessel');
        await enter('input', 'New host');
        await enter('textarea', 'private invitation');
        await click('How do I get an invitation');
        assert.match(query('.connections-guide')!.textContent!, /pair-invite/);
        await click('Back to adding');
        assert.equal((query('input') as HTMLInputElement).value, 'New host');
        assert.equal((query('textarea') as HTMLTextAreaElement).value, 'private invitation');
        await click('Existing credential');
        await enter('textarea', 'private credential');
        await click('New invitation');
        assert.equal((query('textarea') as HTMLTextAreaElement).value, 'private invitation');
        await React.act(async () => query('form')!.dispatchEvent(new dom.window.Event('submit', {bubbles: true, cancelable: true})));
        assert.equal(requests, 1);
        assert.match(query('.connections-notice')!.textContent!, /outcome uncertain/);
        assert.equal((query('textarea') as HTMLTextAreaElement).value, 'private invitation');
        assert.equal((query('button[form="vessel-add-form"]') as HTMLButtonElement).disabled, true);
    } finally {
        await React.act(async () => root.unmount());
        globalThis.fetch = oldFetch;
        Object.assign(globalThis, saved);
        dom.window.close();
    }
});
