import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {spawnSync} from 'node:child_process';
import {mkdtempSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {composer} from '../resources/js/composer.js';
import {voyageSettings} from '../resources/js/voyage-settings.js';

const until = async predicate => {
    for (let i = 0; i < 100; i++) {
        if (predicate()) return;
        await new Promise(resolve => setTimeout(resolve, 5));
    }
    assert.fail('Setup did not reach the expected state');
};

function fixture(t, refreshStatus) {
    const compiled = mkdtempSync(join(tmpdir(),'helm-oauth-setup-'));
    t.after(() => rmSync(compiled,{recursive:true,force:true}));
    const code = `require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); echo view('livewire.console',['vessels'=>collect(),'tenantId'=>'oauth-test'])->render();`;
    const rendered = spawnSync('php',['-r',code],{cwd:new URL('..',import.meta.url),encoding:'utf8',env:{...process.env,VIEW_COMPILED_PATH:compiled}});
    assert.equal(rendered.status,0,rendered.stderr);
    const dom = new JSDOM(rendered.stdout,{url:'https://console.example'});
    t.after(() => dom.window.close());
    for (const key of ['window','document','localStorage','Event']) globalThis[key] = dom.window[key];
    const root = document.querySelector('#helm-client'), field = id => root.querySelector(`#${id}`);
    const binding = {account_id:'oauth-account',connection_id:'oauth-connection',identity_generation:1,connection_revision:1,transport:'chatgpt_oauth'};
    const seen = [];
    let availability = 'expired';
    const client = {socket:new dom.window.EventTarget(),exchange(envelope) {
        const command = envelope.command;
        seen.push(command);
        let result;
        if (command.op === 'capabilities') result = {vessel_id:'vessel-identity',scope:'owner',workspaces:[{name:'Home',path:'/home/psi'}]};
        else if (command.op === 'accounts') result = {accounts:[{id:binding.account_id,connection_id:binding.connection_id,identity_generation:1,state:'ready',availability,label:'Personal ChatGPT'}],connections:[{id:binding.connection_id,revision:1,transports:['chatgpt_oauth'],label:'ChatGPT'}]};
        else if (command.op === 'profiles') result = {revision:1,can_manage:true,default_profile_id:'profile',profiles:[{id:'profile',name:'Default',account:binding,model:'gpt-6-sol',reasoning_effort:null,service_tier:null}]};
        else if (command.op === 'account_usage') {
            if (refreshStatus === 'available' || refreshStatus === 'foreign_available') availability = 'available';
            if (refreshStatus === 'sign_in_required') availability = 'refresh_pending_or_uncertain';
            result = {account:refreshStatus === 'foreign_available' ? {...binding,account_id:'different-account'} : binding,refresh_status:refreshStatus === 'foreign_available' ? 'available' : refreshStatus,snapshot:null};
        } else throw new Error(`Unexpected ${command.op}`);
        return Promise.resolve({protocol:1,error:null,outcome_unknown:false,result});
    }};
    const fleet = {connections:new Map([['vessel',{id:'vessel',vessel_id:'vessel-identity',name:'Vessel',client,voyages:[]}]])};
    let selection = {vessel:'vessel',session_id:null,incarnation:null};
    const select = (vessel,session_id) => { selection = {...selection,vessel,session_id}; };
    const composition = composer(root,{fleet,current:() => selection,select,notice:message => assert.fail(message)});
    const captureDraft = () => {
        const origin = composition.capture();
        return origin?.key && composition.active ? {...origin,workspace:composition.active.document.target.workspace,target:structuredClone(composition.active.document.target)} : null;
    };
    voyageSettings(root,fleet,{current:() => selection,select,apply:() => assert.fail('Inference must not run'),draft:(vessel,workspace) => composition.newChat(vessel,workspace),captureDraft,prepared:() => {}});
    return {field,binding,seen};
}

test('explicit Flux action refreshes one expired OAuth account and retains its profile binding',async t => {
    const f = fixture(t,'available');
    f.field('new-voyage').click();
    await until(() => f.seen.some(command => command.op === 'profiles'));
    f.field('setup-profile-open').click();
    await until(() => !f.field('setup-expired-accounts').hidden);
    assert.equal(f.field('edit-profile ui-option[data-settings-option]').hasAttribute('disabled'),true);
    assert.equal(f.field('edit-expired-account').tagName,'UI-SELECT');
    f.field('setup-refresh-oauth').click();
    await until(() => f.seen.some(command => command.op === 'account_usage'));
    assert.deepEqual(f.seen.filter(command => command.op === 'account_usage').map(({account,workspace,refresh}) => ({account,workspace,refresh})),[{account:f.binding,workspace:'/home/psi',refresh:true}]);
    await until(() => f.field('setup-expired-accounts').hidden && !f.field('edit-save').disabled);
    assert.equal(f.field('edit-profile ui-option[data-settings-option]').hasAttribute('disabled'),false);
    assert.equal(f.seen.some(command => command.op === 'start_account'),false);
});

test('uncertain expired OAuth refresh reports host recovery and does not retry',async t => {
    const f = fixture(t,'sign_in_required');
    f.field('new-voyage').click();
    await until(() => f.seen.some(command => command.op === 'profiles'));
    f.field('setup-profile-open').click();
    await until(() => !f.field('setup-expired-accounts').hidden);
    f.field('setup-refresh-oauth').click();
    await until(() => f.seen.some(command => command.op === 'account_usage') && f.field('setup-expired-accounts').hidden && /pending or uncertain|Reauthenticate/i.test(f.field('setup-oauth-status').textContent));
    assert.equal(f.seen.filter(command => command.op === 'account_usage').length,1);
    assert.equal(f.field('edit-save').disabled,true);
    assert.equal(f.field('setup-expired-accounts').hidden,true);
});

test('foreign OAuth refresh response cannot be reported as a confirmed recovery',async t => {
    const f = fixture(t,'foreign_available');
    f.field('new-voyage').click();
    await until(() => f.seen.some(command => command.op === 'profiles'));
    f.field('setup-profile-open').click();
    await until(() => !f.field('setup-expired-accounts').hidden);
    f.field('setup-refresh-oauth').click();
    await until(() => f.seen.some(command => command.op === 'account_usage') && /could not be confirmed|unconfirmed/i.test(f.field('setup-oauth-status').textContent));
    assert.doesNotMatch(f.field('setup-oauth-status').textContent,/sign-in is available/i);
    assert.equal(f.seen.filter(command => command.op === 'account_usage').length,1);
});
