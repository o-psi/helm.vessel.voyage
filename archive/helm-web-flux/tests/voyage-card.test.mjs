import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {readFileSync} from 'node:fs';
import {cardStatus, decorateCard} from '../resources/js/voyage-card.js';

test('card colors and motion follow lifecycle, never cached or offline activity', () => {
    const item = {state:'live', catalogue:{summary:{run_state:'running'}}};
    assert.deepEqual(cardStatus(item, true), {label:'Running',tone:'active',animated:true});
    assert.equal(cardStatus(item, false).animated, false);
    item.catalogue.stale = true;
    assert.equal(cardStatus(item, true).label, 'Cached');
    assert.equal(cardStatus(item, true, {run:{state:'completed'}}).tone, 'success');
    item.catalogue.stale = false;
    for (const state of ['suspended','stopped','unavailable','relinquished','cleanup_unconfirmed']) {
        assert.equal(cardStatus({...item,state}, true).animated, false);
    }
    for (const [state, tone, animated] of [['starting','active',true],['cancelling','warning',true],['failed','error',false],['idle','muted',false],['completed','success',false]]) {
        assert.deepEqual(cardStatus(item,true,{run:{state}}),{label:state[0].toUpperCase()+state.slice(1),tone,animated});
    }
    assert.equal(cardStatus({state:'live'},true).animated,true);
    assert.equal(cardStatus({},true).label,'Unknown');
});

test('card keeps title and metadata while adding a decorative wheel and readable status', () => {
    const dom = new JSDOM('<button data-current><div data-content><span data-label>A voyage</span><span data-vessel-label>Host</span></div></button>');
    globalThis.document = dom.window.document;
    const control = document.querySelector('button');
    decorateCard(control, {label:'Running',tone:'active',animated:true});
    assert.equal(control.dataset.statusTone,'active');
    assert.ok(control.hasAttribute('data-current'));
    assert.ok(control.hasAttribute('data-animated'));
    assert.equal(control.querySelector('[data-card-status]').textContent,'Running');
    assert.equal(control.querySelector('[data-card-indicator]').getAttribute('aria-hidden'),'true');
    assert.equal(control.querySelector('[data-label]').textContent,'A voyage');
    assert.equal(control.querySelector('[data-vessel-label]').textContent,'Host');
    const indicator = control.querySelector('[data-card-indicator]');
    decorateCard(control, {label:'Cancelling',tone:'warning',animated:true});
    assert.equal(control.querySelector('[data-card-indicator]'), indicator);
    assert.equal(control.querySelectorAll('[data-card-status]').length, 1);
    assert.equal(control.querySelector('[data-card-status]').textContent, 'Cancelling');
    decorateCard(control, {label:'Completed',tone:'success',animated:false});
    assert.equal(control.hasAttribute('data-animated'), false);
    assert.equal(control.querySelector('[data-card-indicator]'), indicator);
    dom.window.close();
    delete globalThis.document;
});

test('card animation has reduced-motion and pointer-safe border treatment', () => {
    const css=readFileSync(new URL('../resources/css/console.css',import.meta.url),'utf8');
    assert.match(css, /prefers-reduced-motion: reduce/);
    assert.match(css, /animation: none; transition: none/);
    assert.match(css, /pointer-events: none/);
    assert.match(css, /mask-composite: exclude/);
    assert.doesNotMatch(css, /inset 3px/);
});
