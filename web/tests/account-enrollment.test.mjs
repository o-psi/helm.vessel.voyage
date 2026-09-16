import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {spawnSync} from 'node:child_process';
import {mkdtempSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {accountEnrollment} from '../resources/js/account-enrollment.js';

const tick = () => new Promise(resolve=>setTimeout(resolve,0));
function fixture() {
    const compiled=mkdtempSync(join(tmpdir(),'helm-enrollment-views-'));
    let rendered;
    try {
        rendered=spawnSync('php',['-r',`require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); if(realpath(config('view.compiled')) !== realpath(getenv('VIEW_COMPILED_PATH'))) exit(1); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); echo view('console.account-enrollment')->render();`],{cwd:new URL('..',import.meta.url),encoding:'utf8',env:{...process.env,VIEW_COMPILED_PATH:compiled}});
    } finally {rmSync(compiled,{recursive:true,force:true});}
    assert.equal(rendered.status,0,rendered.stderr);
    const dom=new JSDOM(`<main data-tenant-id="t">${rendered.stdout}</main>`,{url:'https://helm.test'});
    dom.window.document.querySelector('#enrollment-label').value='Personal';
    globalThis.localStorage=dom.window.localStorage;
    Object.defineProperty(dom.window.navigator,'locks',{value:{request:async (_key,_options,fn)=>fn({})}});
    const root=dom.window.document.querySelector('main'), calls=[], refreshed=[];
    const f={state:'pending',uri:'https://auth.openai.com/codex/device',fail:false};
    const client={socket:new dom.window.EventTarget(), async exchange({command:c}) {
        calls.push(c);
        if(c.op==='enroll_account') {f.enrollment=c.enrollment_id;assert.ok(localStorage.length,'public intent exists before effect');if(f.fail)throw Error('private provider diagnostic');}
        const result=c.op==='capabilities'?{vessel_id:'v',scope:'owner'}:c.op==='accounts'?{connections:[{id:'p',label:'ChatGPT',transports:['chatgpt_oauth']}]}:c.op==='private_account_enrollment'?{status:{enrollment_id:f.enrollment,state:f.state,expires_at:Math.floor(Date.now()/1000)+60,account_id:f.state==='succeeded'?'account':null},user_code:'ABCD-1234',verification_uri:f.uri}:{};
        return {protocol:1,outcome_unknown:false,result};
    }};
    const connection={id:'c',vessel_id:'v',client};
    const ui=accountEnrollment(root,{context:()=>({connection,workspace:'/work'}),refreshed:id=>refreshed.push(id)});
    const $=id=>root.querySelector('#enrollment-'+id);
    return {dom,root,calls,f,connection,ui,$,refreshed};
}
test('ChatGPT enrollment persists only public identities, checks without replay and refreshes success',async()=>{
    const x=fixture();await x.ui.open();x.$('start').click();await tick();
    assert.equal(x.$('code').textContent,'ABCD-1234');
    assert.equal(x.$('link').href,'https://auth.openai.com/codex/device');
    const stored=localStorage.getItem(localStorage.key(0));
    assert.doesNotMatch(stored,/ABCD|verification|user_code|token/);
    x.$('start').click();await tick();assert.equal(x.calls.filter(c=>c.op==='enroll_account').length,1);
    x.f.state='succeeded';x.$('check').click();await tick();
    assert.equal(x.$('code').textContent,'');assert.equal(localStorage.length,0);assert.deepEqual(x.refreshed,['account']);
    const start=x.calls.find(c=>c.op==='enroll_account'),resolve=x.calls.find(c=>c.op==='resolve_account_enrollment');
    assert.deepEqual({...resolve,op:'enroll_account'},start);x.ui.hide();
});
test('uncertain start is resolved, never replayed; cancel has a stable identity',async()=>{
    const x=fixture();await x.ui.open();x.f.fail=true;x.$('start').click();await tick();
    assert.match(x.$('status').textContent,/could not be confirmed/);assert.doesNotMatch(x.root.textContent,/private provider diagnostic/);
    x.$('check').click();await tick();assert.equal(x.calls.filter(c=>c.op==='enroll_account').length,1);
    x.$('cancel').click();await tick();x.$('cancel').click();await tick();
    const cancels=x.calls.filter(c=>c.op==='cancel_account_enrollment');assert.equal(cancels.length,2);assert.deepEqual(cancels[0],cancels[1]);x.ui.hide();
});
test('private code clears on hide, disconnect, replacement and rejects foreign verification links',async()=>{
    const x=fixture();await x.ui.open();x.$('start').click();await tick();
    x.$('close').click();assert.equal(x.$('code').textContent,'');assert.equal(x.$('link').hasAttribute('href'),false);
    await x.ui.open();x.$('check').click();await tick();
    x.connection.client.socket.dispatchEvent(new x.dom.window.Event('close'));assert.equal(x.$('code').textContent,'');
    x.f.uri='https://attacker.invalid/';x.$('check').click();await tick();assert.equal(x.$('link').hasAttribute('href'),false);
    x.f.uri='https://auth.openai.com/codex/device';x.$('check').click();await tick();
    x.connection.client={};x.ui.changed();assert.equal(x.$('code').textContent,'');x.ui.hide();
});
test('late private response after view close is discarded',async()=>{
    const x=fixture();await x.ui.open();const exchange=x.connection.client.exchange;let deliver;
    x.connection.client.exchange=async r=>r.command.op==='private_account_enrollment'?new Promise(resolve=>{deliver=()=>exchange(r).then(resolve);}):exchange(r);
    x.$('start').click();await tick();x.ui.hide();await deliver();await tick();assert.equal(x.$('code').textContent,'');
});
test('enrollment permission refusal prevents provider effects',async()=>{
    const x=fixture();await x.ui.open();const exchange=x.connection.client.exchange;
    x.connection.client.exchange=async r=>r.command.op==='capabilities'?{protocol:1,outcome_unknown:false,result:{vessel_id:'v',scope:'connection',rights:['account_use']}}:exchange(r);
    x.$('start').click();await tick();assert.match(x.$('status').textContent,/does not permit/);assert.equal(x.calls.some(c=>c.op==='enroll_account'),false);assert.equal(localStorage.length,0);
});
test('cross-tab lock contention refuses a second start',async()=>{
    const x=fixture();await x.ui.open();x.dom.window.navigator.locks.request=async (_key,_options,fn)=>fn(null);
    x.$('start').click();await tick();assert.match(x.$('status').textContent,/Another tab/);assert.equal(x.calls.some(c=>c.op==='enroll_account'),false);
});

test('progressive account setup hides recovery commands until sign-in starts',async()=>{
    const x=fixture();await x.ui.open();
    assert.equal(x.$('setup').hidden,false);assert.equal(x.$('start').hidden,false);
    assert.equal(x.$('check').hidden,true);assert.equal(x.$('cancel').hidden,true);
    assert.equal(x.$('provider-field').hidden,true);
    x.$('start').click();await tick();
    assert.equal(x.$('setup').hidden,true);assert.equal(x.$('start').hidden,true);
    assert.equal(x.$('check').hidden,false);assert.equal(x.$('check').textContent,'I’ve signed in');
    assert.match(x.calls.find(c=>c.op==='enroll_account').alias,/^chatgpt-/);
    x.ui.hide();
});
test('closing the sign-in popover hides its private contents',async()=>{
    const x=fixture();const host=x.dom.window.document.createElement('div');
    host.setAttribute('data-flux-popover','');x.root.append(host);host.append(x.$('panel'));
    let closed=0;host.hidePopover=()=>closed++;
    await x.ui.open();x.$('start').click();await tick();x.$('close').click();
    assert.equal(closed,1);assert.equal(x.$('code').textContent,'');assert.equal(x.$('panel').hidden,true);
});
test('catalogue refusal is distinguished from browser storage failure',async()=>{
    const x=fixture();x.connection.client.exchange=async()=>({protocol:1,outcome_unknown:false,error:'private diagnostic'});
    await x.ui.open();assert.match(x.$('status').textContent,/Vessel could not confirm/);assert.doesNotMatch(x.$('status').textContent,/private diagnostic/);
    const y=fixture();localStorage.setItem('helm-web:enrollment:t:c:v:/work','not-json');
    await y.ui.open();assert.match(y.$('status').textContent,/saved sign-in record/);
});
