import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {mountCapture} from '../../helm/browser-view/capture.mjs';

// Synthetic DOM/canvas only: not evidence of live WebRTC capture or native downloads.
function fixture({attach = true, delayed = false, size = 20, fail = false} = {}) {
    const dom = new JSDOM('<button id="trigger">Capture</button><section></section>', {url: 'https://helm.example'});
    const {window: w} = dom, doc = w.document, draws = [], boxes = [], files = [], urls = [], revoked = [];
    let allowed = true, pending, downloads = 0;
    w.HTMLDialogElement.prototype.showModal = function () { this.open = true; };
    w.HTMLDialogElement.prototype.close = function () { this.open = false; };
    w.HTMLCanvasElement.prototype.getContext = function () {
        return {clearRect() {}, drawImage: (...args) => { if (fail) throw new Error('tainted'); draws.push(args); }, strokeRect: (...args) => boxes.push(args)};
    };
    w.HTMLCanvasElement.prototype.toBlob = function (done) {
        const blob = new w.Blob(['x'.repeat(size)], {type: 'image/png'});
        if (delayed) pending = () => done(blob); else done(blob);
    };
    w.HTMLCanvasElement.prototype.setPointerCapture = function () {};
    w.HTMLCanvasElement.prototype.releasePointerCapture = function () {};
    w.URL.createObjectURL = () => { urls.push('blob:test'); return 'blob:test'; };
    w.URL.revokeObjectURL = url => revoked.push(url);
    w.HTMLAnchorElement.prototype.click = () => { downloads++; };
    const video = {readyState: 2, videoWidth: 640, videoHeight: 480};
    const root = doc.querySelector('section'), trigger = doc.querySelector('button'); trigger.focus();
    const api = mountCapture({root, video, canCapture: () => allowed, onCapture: attach ? file => { files.push(file); } : undefined});
    const dialog = root.querySelector('dialog'), canvas = root.querySelector('canvas');
    canvas.getBoundingClientRect = () => ({left: 0, top: 0, width: 320, height: 240});
    const button = name => root.querySelector(`[data-action="${name}"]`);
    const consent = () => { const input = root.querySelector('input'); input.checked = true; input.dispatchEvent(new w.Event('change')); };
    const key = key => canvas.dispatchEvent(new w.KeyboardEvent('keydown', {key, bubbles: true, cancelable: true}));
    const pointer = (type, x, y) => { const event = new w.Event(type); Object.assign(event, {clientX: x, clientY: y, pointerId: 1, button: 0}); canvas.dispatchEvent(event); };
    return {api, root, doc, trigger, dialog, canvas, video, draws, boxes, files, urls, revoked, button, consent, key, pointer,
        deny: () => { allowed = false; }, resolve: () => pending(), downloads: () => downloads, close: () => { api.dispose(); dom.window.close(); }};
}
const tick = () => new Promise(resolve => setTimeout(resolve, 0));

test('mount never samples media; explicit open freezes one image, no automatic disclosure', () => {
    const f = fixture(); assert.equal(f.draws.length, 0); assert.equal(f.files.length, 0);
    assert.equal(f.api.open(), true); assert.equal(f.dialog.open, true); assert.equal(f.draws.filter(x => x[0] === f.video).length, 1);
    assert.equal(f.api.open(), false); assert.equal(f.button('export').disabled, true);
    assert.match(f.root.textContent, /leaves the private view/); assert.match(f.root.textContent, /not a semantic browser-tool result/);
    f.key(' '); f.key('ArrowRight'); f.key('ArrowDown'); f.key('Enter'); assert.ok(f.boxes.length > 0);
    f.button('clear').click(); assert.equal(f.draws.filter(x => x[0] === f.video).length, 1);
    f.button('cancel').click(); assert.equal(f.canvas.width, 0); assert.equal(f.dialog.open, false); assert.equal(f.doc.activeElement, f.trigger); f.close();
});

test('pointer rectangles use scaled frozen image coordinates; clear removes annotations', () => {
    const f = fixture(); f.api.open();
    f.pointer('pointerdown', 10, 20); f.pointer('pointermove', 80, 100); f.pointer('pointerup', 80, 100);
    assert.deepEqual(f.boxes.at(-1), [20, 40, 140, 160]);
    const count = f.boxes.length; f.button('clear').click(); assert.equal(f.boxes.length, count);
    f.close();
});

test('attachment requires confirmation and explicit action; callback gets PNG File only', async () => {
    const f = fixture(); f.api.open(); f.button('export').click(); await tick(); assert.equal(f.files.length, 0);
    f.consent(); f.button('export').click(); f.button('export').click(); await tick();
    assert.equal(f.files.length, 1); assert.equal(f.files[0].name, 'helm-viewer-capture.png'); assert.equal(f.files[0].type, 'image/png');
    assert.equal(f.dialog.open, false); assert.equal(f.canvas.width, 0); assert.equal(f.urls.length, 0); f.close();
});

test('TUI fallback creates download only after click and revokes the object URL', async () => {
    const f = fixture({attach: false}); f.api.open(); assert.equal(f.urls.length, 0);
    assert.equal(f.button('export').textContent, 'Download capture'); f.button('export').click(); await tick();
    assert.equal(f.downloads(), 1); assert.deepEqual(f.revoked, f.urls); assert.equal(f.canvas.width, 0); f.close();
});

for (const action of ['reset', 'dispose', 'deny', 'cancel']) test(`${action} fences a pending PNG export`, async () => {
    const f = fixture({delayed: true}); f.api.open(); f.consent(); f.button('export').click();
    if (action === 'deny') f.deny(); else if (action === 'cancel') f.button('cancel').click(); else f.api[action]();
    f.resolve(); await tick(); assert.equal(f.files.length, 0); assert.equal(f.urls.length, 0); assert.equal(f.canvas.width, 0); f.close();
});

test('late export cannot clear a new capture after reset', async () => {
    const f = fixture({delayed: true}); f.api.open(); f.consent(); f.button('export').click(); f.api.reset(); f.api.open();
    f.resolve(); await tick(); assert.equal(f.dialog.open, true); assert.equal(f.canvas.width, 640); assert.equal(f.files.length, 0); f.close();
});

for (const size of [0, 2 * 1024 * 1024 + 1]) test(`rejects PNG size ${size} without disclosure`, async () => {
    const f = fixture({size}); f.api.open(); f.consent(); f.button('export').click(); await tick();
    assert.equal(f.files.length, 0); assert.match(f.root.textContent, /at most 2 MiB/); assert.equal(f.dialog.open, true); f.close();
});

test('accepts exact 2 MiB boundary', async () => {
    const f = fixture({size: 2 * 1024 * 1024}); f.api.open(); f.consent(); f.button('export').click(); await tick();
    assert.equal(f.files[0].size, 2 * 1024 * 1024); f.close();
});

test('ineligible, unavailable, failed and disposed captures never attach', () => {
    const f = fixture(); f.deny(); assert.equal(f.api.open(), false); assert.equal(f.draws.length, 0); f.close();
    const g = fixture(); g.video.readyState = 1; assert.equal(g.api.open(), false); g.close(); assert.equal(g.api.open(), false);
    const h = fixture({fail: true}); assert.equal(h.api.open(), false); assert.equal(h.canvas.width, 0); assert.equal(h.files.length, 0); h.close();
});

test('Escape abandons keyboard rectangle; dialog cancellation wipes the image', () => {
    const f = fixture(); f.api.open(); f.key(' '); f.key('ArrowRight'); f.key('Escape');
    f.dialog.dispatchEvent(new f.doc.defaultView.Event('cancel', {cancelable: true}));
    assert.equal(f.dialog.open, false); assert.equal(f.canvas.height, 0); f.close();
});
