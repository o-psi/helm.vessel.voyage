import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {spawnSync} from 'node:child_process';
const dom = new JSDOM('<!doctype html><meta name="csrf-token" content="synthetic-token">',{url:'http://localhost/console',pretendToBeVisual:true});
for (const key of ['window','document','location','localStorage','Event','CustomEvent']) globalThis[key]=dom.window[key];
const {mount,markdown} = await import('../resources/js/console.js');
const pause=ms=>new Promise(r=>setTimeout(r,ms));
async function until(predicate) {for(let i=0;i<100;i++){if(predicate())return;await pause(25);}assert.fail('Timed out waiting for browser state');}
const id='10000000-0000-4000-8000-000000000001', incarnation='10000000-0000-4000-8000-000000000002', run='10000000-0000-4000-8000-000000000003', vessel='10000000-0000-4000-8000-000000000004';

test('Markdown is sanitized, selectable HTML with no remote image or script execution', () => {
    const html=markdown('# Heading\n\n**Bold** `code`\n\n<script>alert(1)</script><img src="https://tracker.invalid/pixel"><a href="javascript:alert(1)">bad</a><iframe src="https://evil.invalid"></iframe>');
    assert.match(html,/<h1>Heading<\/h1>/);assert.match(html,/<strong>Bold<\/strong>/);assert.match(html,/<code>code<\/code>/);
    assert.doesNotMatch(html,/<script|<img|javascript:|<iframe/);
});
test('browser journey: history, live output, submit, approval, question, cancel, reconnect receipts without replay', {timeout:15000}, async () => {
    const root=document.createElement('main');root.id='helm-client';root.dataset.ticketUrl='/console/ticket';root.dataset.socketPath='/console/socket';
    const rendered = spawnSync('php', ['-r', `require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); echo view('livewire.console',['vessels'=>collect(),'tenantId'=>'test'])->render();`], {cwd: new URL('..',import.meta.url), encoding:'utf8'});
    assert.equal(rendered.status,0,rendered.stderr);
    const fixture = document.createElement('div'); fixture.innerHTML = rendered.stdout;
    root.innerHTML = fixture.querySelector('#helm-client').innerHTML;
    root.dataset.vessels=JSON.stringify([{id:'local',name:'Local Vessel',vessel_id:vessel}]);
    document.body.append(root);
    const $=selector=>root.querySelector(selector);
    let revision=1, running=false, uncertain=false, dropNext=false, decisionKind=null, requests=[], sockets=[], access='approval';
    globalThis.fetch=async()=>({ok:true,json:async()=>({ticket:'synthetic.ticket'})});
    class Socket extends dom.window.EventTarget {
        readyState=0;
        constructor(){super();sockets.push(this);setTimeout(()=>{this.readyState=1;this.dispatchEvent(new Event('open'));},0);}
        send(text){const frame=JSON.parse(text);if(frame.type==='authenticate'){setTimeout(()=>this.receive({type:'ready',vessel_id:vessel}),0);return;}
            const command=frame.request.command;requests.push(command);
            let result;
            if(command.op==='catalogue')result=[{session_id:id,name:'Synthetic voyage',state:'live',incarnation}];
            else {
                let value;
                if(command.op==='snapshot')value={session_id:id,revision,access,inference:{account:{account_id:id,connection_id:id,connection_revision:1,identity_generation:1,transport:'openai_responses'},model:'fixture',reasoning_effort:'medium',reasoning_efforts:['low','medium','high'],service_tier:null},name:'Synthetic voyage',message_offset:0,messages:[{message_index:0,role:'user',content:'Hello **Vessel**',projection_truncated:false}],run:running?{run_id:run,state:'running',stream_reconciled:true,live_text:'Streamed response',live_text_offset:0}:null};
                else if(command.op==='set_account_inference'){revision++;value={command_id:command.command_id,status:'applied'};}
                else if(command.op==='set_access'){access=command.access;revision++;value={command_id:command.command_id,status:'applied'};}
                else if(command.op==='decisions')value=decisionKind?[{decision_id:'10000000-0000-4000-8000-000000000005',incarnation,run_id:run,expires_at_ms:Date.now()+60000,request:decisionKind==='approval'?{kind:'approval',approval:{action:'shell',target:'synthetic',reason:'test'}}:{kind:'question',question:{question:'Choose one',options:['First','Second']}}}]:[];
                else if(command.op==='receipt'){value={command_id:command.command_id,status:uncertain?'accepted':'unknown'};}
                else if(['submit','steer','respond','cancel'].includes(command.op)){
                    if(dropNext){dropNext=false;uncertain=true;this.close();return;}
                    revision++;running=command.op!=='cancel';if(command.op==='respond')decisionKind=null;value={command_id:command.command_id,status:'accepted'};
                } else throw Error(`Unexpected operation ${command.op}`);
                result={session_id:id,incarnation,result:value};
            }
            setTimeout(()=>this.receive({type:'reply',request_id:frame.request_id,response:{protocol:1,result,error:null,outcome_unknown:false}}),0);
        }
        receive(frame){if(this.readyState===1)this.dispatchEvent(new dom.window.MessageEvent('message',{data:JSON.stringify(frame)}));}
        close(){if(this.readyState===3)return;this.readyState=3;this.dispatchEvent(new Event('close'));}
    }
    globalThis.WebSocket=Socket;
    try {
        assert.equal($('#cancel').hidden,true,'cancel is hidden before selection');
        mount(root);await until(()=>$('#voyages button'));$('#voyages button').click();await until(()=>!$('#send').disabled);
        assert.match($('#messages').textContent,/Hello Vessel/);
        assert.equal($('#cancel').hidden,true,'cancel is hidden for idle voyage');
        assert.equal($('#prompt').getAttribute('submit'),'enter');
        assert.equal($('#composer').querySelectorAll('[data-flux-popover]').length,3);
        assert.equal($('#edit-form').tagName,'DIV','no nested form inside message composer');
        assert.equal($('#change-inference').closest('[data-flux-modal-trigger]'),null);
        $('#change-reasoning').click();
        assert.equal($('#quick-reasoning').value,'2');
        assert.equal($('#quick-reasoning').tagName,'UI-SLIDER');
        assert.equal($('#edit-service').tagName,'UI-RADIO-GROUP');
        $('#quick-reasoning').value='3';$('#reasoning-save').click();
        await until(()=>requests.some(c=>c.op==='set_account_inference')&&!$('#send').disabled);
        assert.equal(requests.find(c=>c.op==='set_account_inference').reasoning_effort,'high');
        $('#prompt').value='A new message';$('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>requests.some(c=>c.op==='submit')&&!$('#send').disabled);
        assert.equal($('#prompt').value,'');assert.match($('#live-output').textContent,/Streamed response/);assert.equal($('#send').getAttribute('aria-label'),'Steer run');
        assert.equal($('#cancel').hidden,false,'cancel is visible for active run');
        decisionKind='approval';await until(()=>[...$('#decisions').querySelectorAll('button')].some(b=>b.textContent.trim()==='Approve'));
        [...$('#decisions').querySelectorAll('button')].find(b=>b.textContent.trim()==='Approve').click();
        await until(()=>requests.some(c=>c.op==='respond'&&c.response==='approved')&&!$('#send').disabled);
        const approval=requests.find(c=>c.op==='respond');assert.equal(approval.incarnation,incarnation);assert.equal(approval.run_id,run);assert.equal(approval.expected_revision,3);
        decisionKind='question';await until(()=>[...$('#decisions').querySelectorAll('button')].some(b=>b.textContent.trim()==='Second'));
        [...$('#decisions').querySelectorAll('button')].find(b=>b.textContent.trim()==='Second').click();
        await until(()=>requests.some(c=>c.response?.status==='selected')&&!$('#send').disabled);
        assert.deepEqual(requests.find(c=>c.response?.status==='selected').response,{status:'selected',index:1,answer:'Second'});
        $('#cancel').click();await until(()=>requests.some(c=>c.op==='cancel')&&!$('#send').disabled);assert.equal($('#send').getAttribute('aria-label'),'Send');
        assert.equal($('#cancel').hidden,true,'cancel hides after run ends');
        assert.equal($('#access-mode').value,'approval');
        $('#access-mode').value='read-only';$('#access-mode').dispatchEvent(new Event('change'));
        await until(()=>requests.some(c=>c.op==='set_access')&&!$('#send').disabled);
        assert.equal($('#access-mode').value,'read-only');
        const accessCommand=requests.find(c=>c.op==='set_access');
        assert.equal(accessCommand.incarnation,incarnation);assert.equal(accessCommand.access,'read-only');
        dropNext=true;$('#prompt').value='Uncertain dispatch';$('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>uncertain);assert.ok($('#send').disabled);assert.match($('#pending').textContent,/outcome not confirmed/);
        await until(()=>requests.some(c=>c.op==='receipt')&&!$('#send').disabled);
        assert.equal(requests.filter(c=>c.op==='submit').length,2,'reconnect must not replay the uncertain submit');
        assert.equal($('#pending').textContent,'');assert.equal($('#prompt').value,'Uncertain dispatch');
        assert.ok(sockets.length>=2);assert.match($('#notice').textContent,/Receipt/);
    } finally {window.dispatchEvent(new Event('pagehide'));root.remove();}
});
