import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {spawnSync} from 'node:child_process';
import {mkdtempSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
const dom = new JSDOM('<!doctype html><meta name="csrf-token" content="synthetic-token">',{url:'http://localhost/console',pretendToBeVisual:true});
for (const key of ['window','document','location','localStorage','Event','CustomEvent']) globalThis[key]=dom.window[key];
const {mount,markdown} = await import('../resources/js/console.js');
const pause=ms=>new Promise(r=>setTimeout(r,ms));
async function until(predicate, describe=()=> 'Timed out waiting for browser state') {for(let i=0;i<100;i++){if(predicate())return;await pause(25);}assert.fail(describe());}
const id='10000000-0000-4000-8000-000000000001', incarnation='10000000-0000-4000-8000-000000000002', run='10000000-0000-4000-8000-000000000003', vessel='10000000-0000-4000-8000-000000000004';

test('Markdown is sanitized, selectable HTML with no remote image or script execution', () => {
    const html=markdown('# Heading\n\n**Bold** `code`\n\n<script>alert(1)</script><img src="https://tracker.invalid/pixel"><a href="javascript:alert(1)">bad</a><iframe src="https://evil.invalid"></iframe>');
    assert.match(html,/<h1>Heading<\/h1>/);assert.match(html,/<strong>Bold<\/strong>/);assert.match(html,/<code>code<\/code>/);
    assert.doesNotMatch(html,/<script|<img|javascript:|<iframe/);
});
test('browser journey: history, live output, submit, approval, question, cancel, reconnect receipts without replay', {timeout:120000}, async (t) => {
    const compiled=mkdtempSync(join(tmpdir(),'helm-console-views-'));
    t.after(()=>rmSync(compiled,{recursive:true,force:true}));
    let id='10000000-0000-4000-8000-000000000001', rejectCreate=false, loseCreate=false, createdProcess;

    const root=document.createElement('main');root.id='helm-client';root.dataset.ticketUrl='/console/ticket';
    const rendered = spawnSync('php', ['-r', `require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); echo view('livewire.console',['vessels'=>collect(),'tenantId'=>'test'])->render();`], {cwd: new URL('..',import.meta.url), encoding:'utf8',env:{...process.env,VIEW_COMPILED_PATH:compiled}});
    assert.equal(rendered.status,0,rendered.stderr);
    const fixture = document.createElement('div'); fixture.innerHTML = rendered.stdout;
    root.innerHTML = fixture.querySelector('#helm-client').innerHTML;
    root.dataset.vessels=JSON.stringify([{id:'local',name:'Local Vessel',vessel_id:vessel}]);
    document.body.append(root);
    const $=selector=>root.querySelector(selector);
    let revision=1, cursor=0, liveText='Streamed response', previews=[], reasoning=[], running=false, failed=false, uncertain=false, dropNext=false, decisionKind=null, requests=[], sockets=[], access='approval', delaySnapshot=false, rejectNext=false, receiptKnown=true;
    globalThis.fetch=async()=>({ok:true,json:async()=>({token:'a'.repeat(64),expires_at_ms:Date.now()+120000,vessel_id:vessel,url:'wss://vessel.example/v1/vessel/browser-socket'})});



    class Socket extends dom.window.EventTarget {
        readyState=0;
        constructor(url,protocol){super();assert.equal(url.href,'wss://vessel.example/v1/vessel/browser-socket');assert.equal(protocol,'voyage.vessel.v1');this.protocol=protocol;sockets.push(this);setTimeout(()=>{this.readyState=1;this.dispatchEvent(new Event('open'));},0);}
        send(text){const frame=JSON.parse(text);if(frame.type==='authenticate'){setTimeout(()=>this.receive({type:'hello',protocol:1,socket_id:incarnation,vessel_id:vessel}),0);return;}
            if(frame.type==='subscribe'){this.subscription=frame.request_id;this.after=frame.request.subscriptions[0].after;return;}
            if(frame.type==='unsubscribe'){this.subscription=null;return;}
            const command=frame.request.command;requests.push(command);
            let result;
            if(command.op==='submit' && rejectNext){rejectNext=false;setTimeout(()=>this.receive({type:'reply',request_id:frame.request_id,response:{protocol:1,result:null,error:'rejected fixture',outcome_unknown:false}}),0);return;}
            if(command.op==='catalogue')result=[{session_id:id,name:'Synthetic voyage',state:'live',incarnation}];

            else if(command.op==='inspect')result={session_id:id,incarnation,workspace:'/fixture'};
            else if(command.op==='capabilities')result={vessel_id:vessel,scope:'owner',workspaces:[{name:'Fixture',path:'/fixture'}]};
            else if(command.op==='profiles')result={revision:1,can_manage:true,default_profile_id:'personal',profiles:[{id:'personal',name:'Everyday',account:{account_id:id,connection_id:id,identity_generation:1,connection_revision:1,transport:'openai_responses'},model:'fixture',reasoning_effort:'high',service_tier:null},{id:'work',name:'Work',account:{account_id:'alternate-account',connection_id:id,identity_generation:2,connection_revision:1,transport:'openai_responses'},model:'fixture',reasoning_effort:'low',service_tier:null}]};
            else if(command.op==='account_models')result={account:command.account,models:[{id:'fixture',is_default:true,reasoning_efforts:['low','high'],service_tiers:['priority']}]};
            else if(command.op==='start_account'){
                if(rejectCreate){rejectCreate=false;setTimeout(()=>this.receive({type:'reply',request_id:frame.request_id,response:{protocol:1,result:null,error:'rejected create',outcome_unknown:false}}),0);return;}
                id=command.session_id;running=false;createdProcess={session_id:id,incarnation,workspace:command.workspace};result=createdProcess;
                if(loseCreate){loseCreate=false;this.close();return;}
            }
            else if(command.op==='resolve_start_account')result={command_id:command.command_id,session_id:command.session_id,status:'created',process:createdProcess};
            else if(command.op==='accounts')result={accounts:[{id,connection_id:id,identity_generation:1,label:'Personal account',state:'ready',availability:'available'},{id:'alternate-account',connection_id:id,identity_generation:2,label:'Work account',state:'ready',availability:'available'},{id:'unavailable-account',connection_id:id,identity_generation:1,label:'Unavailable account',state:'ready',availability:'unavailable'}],connections:[{id,revision:1,transports:['openai_responses']}]};
            else {
                let value;
                if(command.op==='host_browser')value={status:{available:false,running:false,binding:null}};
                else if(command.op==='snapshot')value={session_id:id,revision,observation_cursor:cursor,access,inference:{account:{account_id:id,connection_id:id,connection_revision:1,identity_generation:1,transport:'openai_responses'},model:'fixture',reasoning_effort:'medium',reasoning_efforts:['low','medium','high'],service_tier:null},name:'Synthetic voyage',message_offset:0,messages:[{message_index:0,role:'user',content:'Hello **Vessel**',projection_truncated:false},{message_index:1,role:'assistant',content:'',tool_calls:[{id:'call-1',function:{name:'read_file',arguments:'{}'}}]},{message_index:2,role:'tool',tool_call_id:'call-1',tool_success:true,created_at:'2026-09-16T12:34:00Z',content:'Synthetic tool output'},{message_index:3,role:'assistant',content:'A readable answer.'}],run:running?{run_id:run,state:'running',stream_reconciled:true,live_text:liveText,live_text_offset:0,tool_previews:previews,reasoning_previews:reasoning}:failed?{run_id:run,state:'failed',failure_summary:'Provider request failed.',provider_attempts:[{http_status:400,model:'fixture'}],partial_text:''}:null};
                else if(command.op==='set_account_inference'){revision++;value={command_id:command.command_id,status:'applied'};}
                else if(command.op==='set_access'){access=command.access;revision++;value={command_id:command.command_id,status:'applied'};}
                else if(command.op==='decisions')value=decisionKind?[{decision_id:'10000000-0000-4000-8000-000000000005',incarnation,run_id:run,expires_at_ms:Date.now()+60000,request:decisionKind==='root_grant'?{kind:'root_grant',root_grant:{path:'/fixture/config',permission:'write',lifetime:'current_run',reason:'Configure Boost'}}:decisionKind==='approval'?{kind:'approval',approval:{action:'shell',target:'synthetic',reason:'test'}}:{kind:'question',question:{question:'Choose one',options:['First','Second']}}}]:[];
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
        window.matchMedia=()=>({matches:false});
        $('#host-browser-toggle').click();await until(()=>requests.some(c=>c.op==='host_browser'));
        const oldSocket=sockets.at(-1),oldBrowserCalls=requests.filter(c=>c.op==='host_browser').length;
        oldSocket.close();
        await until(()=>sockets.at(-1)!==oldSocket&&requests.filter(c=>c.op==='host_browser').length>oldBrowserCalls);
        assert.equal($('#host-browser-panel').hidden,false,'viewer remounts after socket replacement');
        assert.equal(requests.filter(c=>c.op==='host_browser'&&c.operation.action==='input').length,0,'reconnect never replays input');
        $('#host-browser-close').click();await until(()=>!$('#send').disabled);
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
        failed=true; observe(); await until(()=>!$('#run-failure').hidden);
        assert.match($('#run-failure-detail').textContent,/Provider request failed\.\nHTTP 400 · fixture/);
        assert.equal($('#composer-model').textContent,'fixture','selected model is visible after the run fails');
        failed=false; observe(); await until(()=>$('#run-failure').hidden);
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
        assert.ok($('#prompt').contains($('#change-setup')),'Setup is the composer entry point');
        assert.ok($('#setup-dialog').contains($('#edit-form')),'settings have their own responsive surface');
        assert.equal($('#edit-form').closest('form'),null,'setup fields cannot submit the message form');
        $('#change-setup').click();
        await until(()=>!$('#setup-overview').hidden);
        assert.equal($('#setup-dialog').querySelector('dialog')?.hasAttribute('open'),true);
        assert.equal($('#setup-location-open').hidden,true,'existing voyage location stays fixed');
        $('#setup-profile-open').click();
        await until(()=>!$('#setup-profiles').hidden && !$('#edit-save').disabled);
        assert.equal($('#edit-account-section').hidden,true);
        assert.equal($('#edit-model-section').hidden,true);
        assert.equal($('#edit-service-section').hidden,true);
        assert.equal($('#edit-form').tagName,'DIV','no nested form inside message composer');
        assert.ok($('#setup-profile-list button[data-selected]'),'a profile is selected in the list');
        assert.match($('#edit-profile-summary').textContent,/fixture/);
        const setupSubmits=requests.filter(c=>c.op==='submit'||c.op==='steer').length;
        $('#setup-profile-search').dispatchEvent(new dom.window.KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}));
        assert.equal(requests.filter(c=>c.op==='submit'||c.op==='steer').length,setupSubmits,'Enter in setup does not send');
        $('#edit-save').click();
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
        decisionKind='root_grant';observe();await until(()=>[...$('#decisions').querySelectorAll('button')].some(b=>b.textContent.trim()==='Grant access for this run'));
        assert.match($('#decisions').textContent,/Current run only/);
        assert.match($('#decisions').textContent,/Read and write/);
        [...$('#decisions').querySelectorAll('button')].find(b=>b.textContent.trim()==='Grant access for this run').click();
        await until(()=>requests.some(c=>c.op==='respond'&&c.response?.root_grant==='approved')&&!$('#send').disabled);
        decisionKind='question';observe();await until(()=>[...$('#decisions').querySelectorAll('button')].some(b=>b.textContent.trim()==='Second'));
        [...$('#decisions').querySelectorAll('button')].find(b=>b.textContent.trim()==='Second').click();
        await until(()=>requests.some(c=>c.response?.status==='selected')&&!$('#send').disabled);
        assert.deepEqual(requests.find(c=>c.response?.status==='selected').response,{status:'selected',index:1,answer:'Second'});
        $('#cancel').click();await until(()=>requests.some(c=>c.op==='cancel')&&!$('#send').disabled);assert.equal($('#send').getAttribute('aria-label'),'Send');
        assert.equal($('#cancel').hidden,true,'cancel hides after run ends');
        $('#change-setup').click();
        await until(()=>!$('#setup-overview').hidden);
        $('#setup-access-open').click();
        assert.equal($('#setup-access').hidden,false,'access opens within Setup');
        assert.equal($('#access-mode').value,'approval');
        $('#access-mode').value='read-only';$('#access-mode').dispatchEvent(new Event('change'));
        await until(()=>requests.some(c=>c.op==='set_access')&&!$('#send').disabled);
        assert.equal($('#access-mode').value,'read-only');
        const accessCommand=requests.find(c=>c.op==='set_access');
        assert.equal(accessCommand.incarnation,incarnation);assert.equal(accessCommand.access,'read-only');
        $('#setup-back').click();
        assert.equal($('#setup-overview').hidden,false);
        $('#setup-close').click();
        rejectNext=true;$('#prompt').value='Rejected dispatch';$('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>!rejectNext&&!$('#send').disabled);assert.equal($('#prompt').value,'Rejected dispatch');
        assert.match($('#notice').textContent,/refused/);
        dropNext=true;receiptKnown=false;$('#prompt').value='Uncertain dispatch';$('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>uncertain);assert.ok($('#send').disabled);assert.match($('#pending').textContent,/outcome not confirmed/);
        await until(()=>requests.some(c=>c.op==='receipt'));
        window.dispatchEvent(new Event('pagehide'));root.remove();
        const replacement=root.cloneNode(false);replacement.innerHTML=fixture.querySelector('#helm-client').innerHTML;root.replaceChildren(...replacement.childNodes);document.body.append(root);
        mount(root);await until(()=>$('#voyages button'));$('#voyages button').click();
        await until(()=>$('#pending').textContent.includes('outcome not confirmed'));assert.equal($('#prompt').value,'','unsent composition is not restored after reload');assert.ok($('#send').disabled);
        receiptKnown=true;
        await until(()=>!$('#send').disabled);
        assert.equal(requests.filter(c=>c.op==='submit').length,3,'reconnect must not replay the uncertain submit');
        assert.equal($('#pending').textContent,'');assert.equal($('#prompt').value,'','accepted receipt safely clears the sent revision');
        assert.ok(sockets.length>=2);assert.match($('#notice').textContent,/Receipt/);

        async function prepareChat(text) {
            $('#new-voyage').click();
            await until(()=>!$('#send').disabled && $('#composer-model').textContent === 'Everyday · fixture' && !$('#setup-overview').hidden);
            assert.equal($('#setup-dialog').querySelector('dialog')?.hasAttribute('open'),true);
            assert.equal($('#setup-location-open').hidden,false,'location editable before first Send');
            assert.match($('#setup-location-summary').textContent,/Local Vessel.*fixture/);
            assert.match($('#conversation-empty').textContent,/first Send creates/);
            assert.equal($('#conversation-empty').hidden,false);
            assert.equal($('#prompt').hasAttribute('disabled'),false);
            $('#setup-done').click();
            await until(()=>!$('#setup-dialog').querySelector('dialog')?.hasAttribute('open'));
            $('#prompt').value=text;$('#prompt').dispatchEvent(new Event('input'));
        }
        await prepareChat('First-send message');
        assert.equal($('#change-setup').disabled,false,'new chats allow setup review before sending');
        assert.match($('#composer-account').textContent,/Personal account/);
        const profileReads=requests.filter(c=>c.op==='profiles').length;
        $('#change-setup').click();
        assert.equal($('#setup-profile-list').querySelector('button'),null,'reloading Setup removes stale selectable profiles');
        await until(()=>!$('#setup-overview').hidden && requests.filter(c=>c.op==='profiles').length>profileReads && /Apply copies/.test($('#edit-status').textContent));
        $('#setup-profile-open').click();
        await until(()=>!$('#setup-profiles').hidden && $('#setup-profile-list button'));
        const profileRow=name=>[...$('#setup-profile-list').querySelectorAll('button')].find(button=>button.querySelector('.setup-choice-title')?.textContent.startsWith(name));
        profileRow('Work').click();
        $('#edit-save').click();
        await until(()=>$('#composer-account').textContent.includes('Work account'),()=>`Profile apply did not settle: saveDisabled=${$('#edit-save').disabled}, status=${$('#edit-status').textContent}, selected=${$('#edit-profile').value}, account=${$('#composer-account').textContent}, overviewHidden=${$('#setup-overview').hidden}, recentOps=${requests.slice(-8).map(c=>c.op).join(',')}`);
        assert.equal($('#setup-overview').hidden,false,'profile apply returns to setup overview');
        $('#setup-profile-open').click();
        await until(()=>!$('#setup-profiles').hidden && !$('#edit-save').disabled);
        assert.equal($('#edit-profile').value,'work','review retains the selected settings');
        assert.equal($('#prompt').value,'First-send message','review preserves the unsent message');
        profileRow('Everyday').click();
        $('#setup-back').click();
        assert.equal($('#edit-profile').value,'work','Back discards an unconfirmed profile choice');
        $('#setup-reasoning-open').click();
        assert.equal($('#setup-reasoning').hidden,false,'reasoning override is reachable in Setup');
        assert.ok($('#setup-reasoning').contains($('#quick-reasoning')));
        const startsBeforeReasoning=requests.filter(c=>c.op==='start_account').length;
        $('#reasoning-save').click();
        assert.equal($('#setup-overview').hidden,false,'applying reasoning returns to overview');
        assert.equal(requests.filter(c=>c.op==='start_account').length,startsBeforeReasoning,'setup changes do not create a voyage');
        $('#setup-close').click();
        assert.equal($('#prompt').value,'First-send message','closing setup preserves the unsent message');
        const submissions=requests.filter(c=>c.op==='submit').length;
        $('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        $('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>requests.filter(c=>c.op==='submit').length===submissions+1 && !$('#send').disabled);
        assert.equal(requests.filter(c=>c.op==='start_account').length,1);
        assert.deepEqual(requests.find(c=>c.op==='start_account').account,{account_id:'alternate-account',connection_id:'10000000-0000-4000-8000-000000000001',identity_generation:2,connection_revision:1,transport:'openai_responses'});
        assert.equal(requests.filter(c=>c.op==='submit').at(-1).session_id,createdProcess.session_id);
        assert.equal(requests.filter(c=>c.op==='submit').at(-1).prompt,'First-send message');
        await prepareChat('Keep rejected creation text');rejectCreate=true;
        $('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>!rejectCreate && !$('#send').disabled);
        assert.equal($('#prompt').value,'Keep rejected creation text');
        assert.match($('#notice').textContent,/Creation was rejected/);
        assert.equal(requests.filter(c=>c.op==='submit').length,submissions+1);
        loseCreate=true;$('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>!loseCreate && $('#pending-creations button'));
        const starts=requests.filter(c=>c.op==='start_account').length;
        assert.equal($('#prompt').value,'Keep rejected creation text');
        await until(()=>!$('#send').disabled);
        $('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>$('#notice').textContent.includes('unconfirmed'));
        assert.equal(requests.filter(c=>c.op==='start_account').length,starts,'lost create response is never replayed');
        $('#pending-creations button').click();
        await until(()=>!$('#pending-creations button') && !$('#send').disabled);
        assert.equal($('#prompt').value,'Keep rejected creation text','creation reconciliation retains draft linkage');
        assert.equal(requests.filter(c=>c.op==='submit').length,submissions+1,'reconciliation does not auto-send');
        $('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>requests.filter(c=>c.op==='submit').length===submissions+2 && !$('#send').disabled);
        assert.equal(requests.filter(c=>c.op==='start_account').length,starts);
        await prepareChat('Retry submission, not creation');rejectNext=true;
        $('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>!rejectNext && !$('#send').disabled);
        assert.equal($('#prompt').value,'Retry submission, not creation');
        const acceptedStarts=requests.filter(c=>c.op==='start_account').length;
        const rejectedSubmits=requests.filter(c=>c.op==='submit').length;
        $('#composer').dispatchEvent(new Event('submit',{cancelable:true}));
        await until(()=>requests.filter(c=>c.op==='submit').length===rejectedSubmits+1 && !$('#send').disabled);
        assert.equal(requests.filter(c=>c.op==='start_account').length,acceptedStarts,'retry after rejected submission reuses the created chat');
    } finally {window.dispatchEvent(new Event('pagehide'));root.remove();}
});
