import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {conversationScroll} from '../resources/js/conversation-scroll.js';

function fixture() {
    const dom = new JSDOM('<div id="conversation" tabindex="0"><div id="messages"></div><div id="output"></div></div>');
    const win = dom.window, viewport = win.document.querySelector('#conversation');
    let height = 1000, size = 200, top = 800, frames = new Map(), serial = 0, observer;
    win.requestAnimationFrame = fn => { frames.set(++serial, fn); return serial; };
    win.cancelAnimationFrame = id => frames.delete(id);
    win.ResizeObserver = class { constructor(fn) {this.fn=fn; observer=this;this.targets=[];} observe(el){this.targets.push(el);} disconnect(){this.disconnected=true;} };
    Object.defineProperties(viewport, {
        scrollHeight:{get:()=>height}, clientHeight:{get:()=>size},
        scrollTop:{get:()=>top,set:value=>{top=Math.max(0,Math.min(value,height-size));}},
    });
    const controller = conversationScroll(viewport, win.document.createElement('button'));
    return {win,viewport,controller,get observer(){return observer;},
        height(value){height=value;}, size(value){size=value;},
        flush(){const pending=[...frames.values()];frames.clear();pending.forEach(fn=>fn());},
        scroll(value){viewport.scrollTop=value;viewport.dispatchEvent(new win.Event('scroll'));},
        close(){controller.dispose();dom.window.close();},
    };
}
test('follows streaming mutations, delayed image growth and viewport resizing', async () => {
    const f=fixture(); try {
        f.flush(); assert.equal(f.observer.targets.length,2);
        f.height(1500); f.viewport.querySelector('#output').textContent='streaming';
        await Promise.resolve(); f.flush(); assert.equal(f.viewport.scrollTop,1300);
        f.height(1700); f.observer.fn(); f.observer.fn(); f.flush(); assert.equal(f.viewport.scrollTop,1500);
        f.size(100); f.observer.fn(); f.flush(); assert.equal(f.viewport.scrollTop,1600);
    } finally {f.close();}
});
test('queued updates respect upward scrolling; bottom and jump resume following', () => {
    const f=fixture(); try {
        f.flush(); f.height(1200); f.observer.fn(); f.scroll(600); f.flush();
        assert.equal(f.viewport.scrollTop,600); assert.equal(f.viewport.nextElementSibling.hidden,false);
        f.height(1500); f.observer.fn(); f.flush(); assert.equal(f.viewport.scrollTop,600);
        f.scroll(1300); f.height(1600); f.observer.fn(); f.flush(); assert.equal(f.viewport.scrollTop,1400);
        f.scroll(100); f.viewport.nextElementSibling.click(); f.flush(); assert.equal(f.viewport.scrollTop,1400);
    } finally {f.close();}
});
test('upward intent cancels pending follow before scroll event; reset and cleanup', () => {
    const f=fixture(); try {
        f.flush(); f.height(1400); f.observer.fn();
        f.viewport.dispatchEvent(new f.win.WheelEvent('wheel',{deltaY:-10}));
        f.flush(); assert.equal(f.viewport.scrollTop,800);
        f.controller.reset(); f.flush(); assert.equal(f.viewport.scrollTop,1200);
        f.height(1600); f.observer.fn(); f.controller.dispose(); f.flush();
        assert.equal(f.viewport.scrollTop,1200); assert.equal(f.observer.disconnected,true);
    } finally {f.close();}
});
test('prepending history preserves reading position and stays detached', () => {
    const f=fixture(); try {
        f.flush(); f.scroll(200);
        f.controller.preservePrepend(()=>f.height(1400));
        assert.equal(f.viewport.scrollTop,600);
        f.observer.fn(); f.flush(); assert.equal(f.viewport.scrollTop,600);
    } finally {f.close();}
});
