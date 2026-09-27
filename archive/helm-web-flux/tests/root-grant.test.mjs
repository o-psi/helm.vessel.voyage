import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {renderRootGrant} from '../resources/js/root-grant.js';

function fixture(grant) {
    const dom = new JSDOM('<h2></h2><p></p><div></div>');
    const doc = dom.window.document, responses = [];
    const ui = {heading:doc.querySelector('h2'),content:doc.querySelector('p'),actions:doc.querySelector('div'),host:'executing-vessel-id',respond:r=>responses.push(r),button:(label,action)=>{const b=doc.createElement('button');b.textContent=label;b.onclick=action;return b;}};
    const handled = renderRootGrant({kind:'root_grant',root_grant:grant},ui);
    return {...ui,handled,responses,doc};
}
const grant = {path:'/home/owner/.config/helm',permission:'write',lifetime:'current_run',reason:'Configure Boost'};

test('root grant review shows scope and sends distinct approve/deny envelopes',()=>{
    const f=fixture(grant);
    for(const text of [grant.path,'Read and write','Current run only','Not inherited','Configure Boost','executing-vessel-id']) assert.ok(f.content.textContent.includes(text));
    assert.deepEqual(f.responses,[]);
    const buttons=f.actions.querySelectorAll('button');buttons[0].click();buttons[1].click();
    assert.deepEqual(f.responses,[{root_grant:'approved'},{root_grant:'denied'}]);
    assert.ok(fixture({...grant,permission:'read'}).content.textContent.includes('Read only'));
});
test('unsupported or malformed scope has no consent buttons',()=>{
    for(const bad of [null,{}, {...grant,permission:'execute'},{...grant,lifetime:'forever'},{...grant,path:'relative'},{...grant,path:'/tmp/\nforged'},{...grant,reason:''}]) {
        const f=fixture(bad);assert.equal(f.handled,true);assert.equal(f.actions.children.length,0);assert.deepEqual(f.responses,[]);
    }
});
test('untrusted path and reason render as text, never markup',()=>{
    const f=fixture({...grant,path:'/tmp/<img src=x>',reason:'<script>bad()</script>'});
    assert.equal(f.doc.querySelector('img,script'),null);
    assert.ok(f.content.textContent.includes('<script>bad()</script>'));
});
test('generic approvals are not interpreted as filesystem grants',()=>{
    assert.equal(renderRootGrant({kind:'approval'},{}),false);
});
