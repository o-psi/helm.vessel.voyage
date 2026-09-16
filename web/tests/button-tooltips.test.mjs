import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {installButtonTooltips} from '../resources/js/button-tooltips.js';

test('icon controls get hover and focus descriptions, including dynamic controls and profile initials', () => {
    const {window:w} = new JSDOM('<button aria-label="Close modal"><svg></svg></button><button data-flux-profile aria-label="Profile menu">A</button><button>Visible action</button>');
    const d=w.document; installButtonTooltips(d);
    const tip=d.querySelector('[role="tooltip"]');
    for (const button of [...d.querySelectorAll('button')].slice(0,2)) {
        button.dispatchEvent(new w.Event('focusin',{bubbles:true}));
        assert.equal(tip.textContent,button.getAttribute('aria-label'));
        assert.equal(tip.hidden,false);
        assert.equal(button.getAttribute('aria-describedby'),tip.id);
        button.dispatchEvent(new w.KeyboardEvent('keydown',{key:'Escape',bubbles:true}));
        assert.equal(tip.hidden,true);
        assert.equal(button.hasAttribute('aria-describedby'),false);
    }
    const dynamic=d.createElement('button'); dynamic.setAttribute('aria-label','Send'); dynamic.setAttribute('aria-describedby','existing'); d.body.append(dynamic);
    dynamic.dispatchEvent(new w.Event('pointerover',{bubbles:true}));
    assert.equal(tip.textContent,'Send');
    dynamic.dispatchEvent(new w.Event('pointerout',{bubbles:true}));
    assert.equal(dynamic.getAttribute('aria-describedby'),'existing');
    dynamic.title='Steer run · Enter';
    dynamic.dispatchEvent(new w.Event('focusin',{bubbles:true}));
    assert.equal(tip.textContent,'Steer run · Enter');
    dynamic.dispatchEvent(new w.Event('focusout',{bubbles:true}));
    d.querySelectorAll('button')[2].dispatchEvent(new w.Event('pointerover',{bubbles:true}));
    assert.equal(tip.hidden,true);
});


test('picker buttons are inspected without constructing detached custom elements', () => {
    const {window:w} = new JSDOM('<ui-select><button aria-label="Model"><ui-selected>Choose a model</ui-selected><svg><title>Chevron</title></svg></button></ui-select><button aria-label="Send"><svg><title>Send icon</title></svg><span class="sr-only">Send</span><span aria-hidden="true">Hidden</span></button>');
    const d=w.document;
    let constructions=0;
    w.customElements.define('ui-selected', class extends w.HTMLElement {
        constructor() {
            super();
            constructions++;
            assert.ok(this.closest('ui-select'), 'picker constructor requires its parent');
        }
    });
    assert.equal(constructions,1);
    installButtonTooltips(d);
    const tip=d.querySelector('[role="tooltip"]');
    for (const type of ['pointerover','focusin']) {
        d.querySelector('ui-selected').dispatchEvent(new w.Event(type,{bubbles:true}));
        assert.equal(constructions,1, 'text inspection must not clone custom elements');
        assert.equal(tip.hidden,true, 'visible picker text needs no icon tooltip');
    }
    const icon=d.querySelectorAll('button')[1];
    icon.dispatchEvent(new w.Event('focusin',{bubbles:true}));
    assert.equal(tip.hidden,false);
    assert.equal(tip.textContent,'Send');
});
