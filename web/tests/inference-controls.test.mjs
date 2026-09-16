import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {setReasoning, reasoningValue} from '../resources/js/inference-controls.js';

test('reasoning slider maps discrete positions to provider values, including default and current', () => {
    const dom = new JSDOM('<ui-slider id="r"><input type="range"></ui-slider><span id="r-value"></span>');
    globalThis.document = dom.window.document;
    const field = document.getElementById('r');
    setReasoning(field,[{value:'',label:'Provider default'},{value:'low',label:'Low'},{value:'custom',label:'custom · current'}],'custom');
    assert.equal(field.value,'2'); assert.equal(reasoningValue(field),'custom');
    assert.equal(field.getAttribute('max'),'2');
    field.value='0'; field.dispatchEvent(new dom.window.Event('input'));
    assert.equal(reasoningValue(field),'');
    assert.equal(document.getElementById('r-value').textContent,'Provider default');
    assert.equal(field.querySelector('input').getAttribute('aria-valuetext'),'Provider default');
    setReasoning(field,[{value:'',label:'Provider default'}]);
    assert.equal(field.disabled,true); assert.equal(reasoningValue(field),'');
});
