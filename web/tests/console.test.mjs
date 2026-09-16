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
    const root=document.createElement('main');root.id='helm-client';root.dataset.ticketUrl='/console/ticket';
    const rendered = spawnSync('php', ['-r', `require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); echo view('livewire.console',['vessels'=>collect(),'tenantId'=>'test'])->render();`], {cwd: new URL('..',import.meta.url), encoding:'utf8'});
    assert.equal(rendered.status,0,rendered.stderr);
    const fixture = document.createElement('div'); fixture.innerHTML = rendered.stdout;
    root.innerHTML = fixture.querySelector('#helm-client').innerHTML;
    root.dataset.vessels=JSON.stringify([{id:'local',name:'Local Vessel',vessel_id:vessel}]);
    document.body.append(root);
    const $=selector=>root.querySelector(selector);
    let revision=1, cursor=0, liveText='Streamed response', previews=[], reasoning=[], running=false, uncertain=false, dropNext=false, decisionKind=null, requests=[], sockets=[], access='approval', delaySnapshot=false, rejectNext=false, receiptKnown=true;
    globalThis.fetch=async()=>({ok:true,json:async()=>({token:'a'.repeat(64),expires_at_ms:Date.now()+120000,vessel_id:vessel,url:'wss://vessel.example/v1/vessel/browser-socket'})});

    const sharedRecords=new Map();

    class Socket extends dom.window.EventTarget {
        readyState=0;
        constructor(url,protocol){super();assert.equal(url.href,'wss://vessel.example/v1/vessel/browser-socket');assert.equal(protocol,'voyage.vessel.v1');this.protocol=protocol;sockets.push(this);setTimeout(()=>{this.readyState=1;this.dispatchEvent(new Event('open'));},0);}
        send(text){const frame=JSON.parse(text);if(frame.type==='authenticate'){setTimeout(()=>this.receive({type:'hello',protocol:1,socket_id:incarnation,vessel_id:vessel}),0);return;}
            if(frame.type==='subscribe'){this.subscription=frame.request_id;this.after=frame.request.subscriptions[0].after;return;}
            if(frame.type==='unsubscribe'){this.subscription=null;return;}
            const command=frame.request.command;requests.push(command);
            let result;
            if(command.op==='submit' && rejectNext){rejectNext=false;setTimeout(()=>this.receive({type:'reply',request_id:frame.request_id,response:{protocol:1,result:null,error:'rejected fixture',outcome_unknown:false}}),0);return;}
            if(command.op==='drafts') {
                const operation=command.operation; this.drafts ||= sharedRecords;
                if(operation.op==='list')result={drafts:[...this.drafts.values()]};
                else if(operation.op==='get')result=this.drafts.get(operation.draft_id)||null;
                else if(operation.op==='put'){result={draft_id:operation.draft_id,revision:operation.expected_revision+1,document:operation.document};this.drafts.set(operation.draft_id,result);}
                else if(operation.op==='delete'){this.drafts.delete(operation.draft_id);result={deleted:true};}
            }
            else if(command.op==='catalogue')result=[{session_id:id,name:'Synthetic voyage',state:'live',incarnation}];
            else if(command.op==='inspect')result={session_id:id,incarnation,workspace:'/fixture'};
            else if(command.op==='accounts')result={accounts:[{id,connection_id:id,identity_generation:1,label:'Personal account'}],connections:[{id,revision:1,transports:['openai_responses']}]};
            else {
                let value;
                if(command.op==='snapshot')value={session_id:id,revision,observation_cursor:cursor,access,inference:{account:{account_id:id,connection_id:id,connection_revision:1,identity_generation:1,transport:'openai_responses'},model:'fixture',reasoning_effort:'medium',reasoning_efforts:['low','medium','high'],service_tier:null},name:'Synthetic voyage',message_offset:0,messages:[{message_index:0,role:'user',content:'Hello **Vessel**',projection_truncated:false},{message_index:1,role:'assistant',content:'',tool_calls:[{id:'call-1',function:{name:'read_file',arguments:'{}'}}]},{message_index:2,role:'tool',tool_call_id:'call-1',tool_success:true,created_at:'2026-09-16T12:34:00Z',content:'Synthetic tool output'},{message_index:3,role:'assistant',content:'A readable answer.'}],run:running?{run_id:run,state:'running',stream_reconciled:true,live_text:liveText,live_text_offset:0,tool_previews:previews,reasoning_previews:reasoning}:null};
                else if(command.op==='set_account_inference'){revision++;value={command_id:command.command_id,status:'applied'};}
                else if(command.op==='set_access'){access=command.access;revision++;value={command_id:command.command_id,status:'applied'};}
                else if(command.op==='decisions')value=decisionKind?[{decision_id:'10000000-0000-4000-8000-000000000005',incarnation,run_id:run,expires_at_ms:Date.now()+60000,request:decisionKind==='approval'?{kind:'approval',approval:{action:'shell',target:'synthetic',reason:'test'}}:{kind:'question',question:{question:'Choose one',options:['First','Second']}}}]:[];
                else if(command.op==='receipt'){value={command_id:command.command_id,status:uncertain && receiptKnown?'accepted':'unknown'};}
                else if(['submit','steer','respond','cancel'].includes(command.op)){
                    if(dropNext){dropNext=false;uncertain=true;this.close();return;}
                    revision++;running=command.op!=='cancel';if(command.op==='respond')decisionKind=null;value={command_id:command.command_id,status:'accepted'};
                } else throw Error(`Unexpected operation ${command.op}`);
                result={session_id:id,incarnation,result:value};
            }
            setTimeout(()=>this.receive({type:'reply',request_id:frame.request_id,response:{protocol:1,result,error:null,outcome_unknown:false}}),command.op==='snapshot' && delaySnapshot ? 120 : 0);
        }
        receive(frame){if(this.readyState===1)this.dispatchEvent(new dom.window.MessageEvent('message',{data:JSON.stringify(frame)}));}
        close(){if(this.readyState===3)return;this.readyState=3;this.dispatchEvent(new Event('close'));}
    }
    globalThis.WebSocket=Socket;
    try {
        assert.equal($('#cancel').hidden,true,'cancel is hidden before selection');
        mount(root);await until(()=>$('#voyages button'));$('#voyages button').click();await until(()=>!$('#send').disabled);
        const observe = (options={}) => {
            const socket=sockets.at(-1); const previous=socket.subscription;
            cursor++;
            socket.receive({type:'event',subscription_id:previous,event:{protocol:1,session_id:id,incarnation,result:{projection:'public-v1',cursor,latest_cursor:cursor,replay_gap:false,has_more:false,events:[{cursor,session_id:id,kind:'run',revision,run_id:run}],...options},error:null,outcome_unknown:false}});
        };
        running=true;
        previews=[{attempt_id:'attempt',index:0,call_id:'fragment',name:'shell',arguments:'{"command":"ec',truncated:false}];
        reasoning=[{attempt_id:'attempt',index:0,kind:'summary',text:'Disclosed rationale',finalized:false}];
        observe(); await until(()=>$('#live-previews').textContent.includes('Disclosed rationale'));
        assert.match($('#live-previews').textContent,/command/);
        $('#live-previews details').open=true;
        liveText='Streamed response delta'; previews[0].arguments='{"command":"echo hello"}';
        observe(); await until(()=>$('#output-text').textContent.includes('delta'));
        assert.equal($('#live-previews details').open,true,'stable provisional identity preserves disclosure');
        assert.match($('#live-previews').textContent,/echo hello/);
        previews[0].call_id='call-1'; reasoning[0].finalized=true;
        observe(); await until(()=>$('#live-previews').textContent.includes('finalized disclosure'));
        assert.equal($('#live-previews [data-preview-key^="tool:"]'),null,'canonical tool call removes provisional preview');
        const before=requests.filter(r=>r.op==='snapshot').length;
        const socket=sockets.at(-1);
        socket.receive({type:'event',subscription_id:socket.subscription,event:{protocol:1,session_id:id,incarnation,result:{projection:'public-v1',cursor,latest_cursor:cursor,replay_gap:false,events:[]},error:null,outcome_unknown:false}});
        await new Promise(r=>setTimeout(r,30));
        assert.equal(requests.filter(r=>r.op==='snapshot').length,before,'duplicate cursor does not refetch or append');
        observe({replay_gap:true,events:[]}); await until(()=>requests.filter(r=>r.op==='snapshot').length>before && !$('#send').disabled);
        assert.equal(sockets.at(-1).after,cursor,'gap resubscribes at fresh snapshot cursor');
        running=false; previews=[]; reasoning=[]; observe(); await until(()=>$('#live-output').hidden && $('#live-previews').hidden);
        const quietSnapshots=requests.filter(c=>c.op==='snapshot').length;
        await new Promise(r=>setTimeout(r,1100));
        assert.equal(requests.filter(c=>c.op==='snapshot').length,quietSnapshots,'quiet subscriptions do not poll snapshots each second');

        assert.match($('#messages').textContent,/Hello Vessel/);
        await until(()=>$('#composer-account').textContent==='Personal account');
        assert.equal($('#composer-service').textContent,'Default tier');
        assert.equal($('#composer-reasoning').textContent,'medium');
        assert.equal(root.querySelector('[data-flux-header]'),null,'no top bar');
        const group=$('#messages [data-tool-group]');
        const entries=group.querySelectorAll('[data-tool-entry]');assert.equal(entries.length,1);
        const tools=entries[0];assert.equal(tools.open,false);
        assert.match(group.textContent,/1 action/);
        assert.match(tools.textContent,/Done · timing unavailable · Read/);
        assert.equal(group.querySelector('[data-expand-tools]').hidden,true);
        assert.equal(tools.open,false,'tool details close without discarding content');
        tools.open=true;tools.dispatchEvent(new Event('toggle'));
        assert.match(tools.textContent,/Synthetic tool output/);
        assert.match($('#messages article[aria-label="Assistant message"]').textContent,/A readable answer/);
        assert.equal($('#cancel').hidden,true,'cancel is hidden for idle voyage');
        assert.equal($('#prompt').getAttribute('submit'),'enter');
        assert.equal($('#composer').querySelectorAll('[data-flux-popover]').length,5);
        $('#change-account').click();
        assert.equal($('#edit-form').parentElement.id,'account-popover');
        assert.equal($('#edit-account-section').hidden,false);
        assert.equal($('#edit-model-section').hidden,true);
        $('#change-inference').click();
        assert.equal($('#edit-form').parentElement.id,'edit-popover');
        assert.equal($('#edit-account-section').hidden,true);
        assert.equal($('#edit-model-section').hidden,false);
        assert.equal($('#edit-service-section').hidden,true);
        assert.equal($('#edit-form').tagName,'DIV','no nested form inside message composer');
        assert.equal($('#change-inference').closest('[data-flux-modal-trigger]'),null);
        $('#change-reasoning').click();
        assert.equal($('#quick-reasoning').value,'2');
        assert.equal($('#quick-reasoning').tagName,'UI-SLIDER');
        assert.equal($('#edit-service').tagName,'UI-RADIO-GROUP');
        $('#quick-reasoning').value='3';$('#reasoning-save').click();
        await until(()=>requests.some(c=>c.op==='set_account_inference')&&!$('#send').disabled);
        assert.equal(requests.find(c=>c.op==='set_account_inference').reasoning_effort,'high');
        delaySnapshot=true;
        const snapshotsBefore=requests.filter(c=>c.op==='snapshot').length; observe();
        await until(()=>requests.filter(c=>c.op==='snapshot').length>snapshotsBefore && !$('#send').disabled);
        $('#prompt').value='A new message';$('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>requests.some(c=>c.op==='submit')&&!$('#send').disabled);
        delaySnapshot=false;assert.equal($('#prompt').value,'');assert.match($('#live-output').textContent,/Streamed response/);assert.equal($('#send').getAttribute('aria-label'),'Steer run');
        assert.equal($('#cancel').hidden,false,'cancel is visible for active run');
        decisionKind='approval';observe();await until(()=>[...$('#decisions').querySelectorAll('button')].some(b=>b.textContent.trim()==='Approve'));
        [...$('#decisions').querySelectorAll('button')].find(b=>b.textContent.trim()==='Approve').click();
        await until(()=>requests.some(c=>c.op==='respond'&&c.response==='approved')&&!$('#send').disabled);
        const approval=requests.find(c=>c.op==='respond');assert.equal(approval.incarnation,incarnation);assert.equal(approval.run_id,run);assert.equal(approval.expected_revision,3);
        decisionKind='question';observe();await until(()=>[...$('#decisions').querySelectorAll('button')].some(b=>b.textContent.trim()==='Second'));
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
        rejectNext=true;$('#prompt').value='Rejected dispatch';$('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>!rejectNext&&!$('#send').disabled);assert.equal($('#prompt').value,'Rejected dispatch');
        assert.match($('#notice').textContent,/refused/);
        dropNext=true;receiptKnown=false;$('#prompt').value='Uncertain dispatch';$('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>uncertain);assert.ok($('#send').disabled);assert.match($('#pending').textContent,/outcome not confirmed/);
        await until(()=>requests.some(c=>c.op==='receipt'));
        window.dispatchEvent(new Event('pagehide'));root.remove();
        const replacement=root.cloneNode(false);replacement.innerHTML=fixture.querySelector('#helm-client').innerHTML;root.replaceChildren(...replacement.childNodes);document.body.append(root);
        mount(root);await until(()=>$('#voyages button'));$('#voyages button').click();
        await until(()=>$('#prompt').value==='Uncertain dispatch');assert.ok($('#send').disabled);
        receiptKnown=true;
        await until(()=>!$('#send').disabled);
        assert.equal(requests.filter(c=>c.op==='submit').length,3,'reconnect must not replay the uncertain submit');
        assert.equal($('#pending').textContent,'');assert.equal($('#prompt').value,'','accepted receipt safely clears the sent revision');
        assert.ok(sockets.length>=2);assert.match($('#notice').textContent,/Receipt/);
    } finally {window.dispatchEvent(new Event('pagehide'));root.remove();}
});
