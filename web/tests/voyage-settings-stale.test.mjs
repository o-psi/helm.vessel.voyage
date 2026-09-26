import test from 'node:test';
import assert from 'node:assert/strict';
import {JSDOM} from 'jsdom';
import {spawnSync} from 'node:child_process';
import {mkdtempSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {voyageSettings} from '../resources/js/voyage-settings.js';

const until = async predicate => {
    for (let i = 0; i < 100; i++) {
        if (predicate()) return;
        await new Promise(resolve => setTimeout(resolve, 5));
    }
    assert.fail('Setup did not reach the expected state');
};

test('setup gives a reload path when a client is lost or replaced during discovery', async t => {
    const compiled = mkdtempSync(join(tmpdir(),'helm-stale-setup-'));
    t.after(() => rmSync(compiled,{recursive:true,force:true}));
    const rendered = spawnSync('php',['-r',`require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); echo view('livewire.console',['vessels'=>collect(),'tenantId'=>'stale-test'])->render();`],{cwd:new URL('..',import.meta.url),encoding:'utf8',env:{...process.env,VIEW_COMPILED_PATH:compiled}});
    assert.equal(rendered.status,0,rendered.stderr);
    const dom = new JSDOM(rendered.stdout,{url:'https://console.example'});
    t.after(() => dom.window.close());
    for (const key of ['window','document','localStorage','Event']) globalThis[key] = dom.window[key];
    const root = document.querySelector('#helm-client'), field = id => root.querySelector(`#${id}`);
    const account = {account_id:'10000000-0000-4000-8000-000000000001',connection_id:'10000000-0000-4000-8000-000000000002',identity_generation:1,connection_revision:1,transport:'openai_responses'};
    const results = {
        capabilities:{vessel_id:'vessel',scope:'owner',workspaces:[{name:'Known',path:'/known'}]},
        accounts:{accounts:[{id:account.account_id,connection_id:account.connection_id,identity_generation:1,state:'ready',availability:'available',label:'Account'}],connections:[{id:account.connection_id,revision:1,transports:[account.transport],label:'Provider'}]},
        profiles:{revision:1,can_manage:true,default_profile_id:'default',profiles:[{id:'default',name:'Default',account,model:'model',reasoning_effort:null,service_tier:null}]},
    };
    const reply = command => ({protocol:1,error:null,outcome_unknown:false,result:command.op === 'account_models' ? {account:command.account,models:[{id:'model',is_default:true}]} : structuredClone(results[command.op])});
    let heldOp = 'capabilities', pending, origin, selection = {vessel:'v'};
    const seen = [];
    const makeClient = label => ({socket:new dom.window.EventTarget(),exchange(envelope) {
        const command = envelope.command; seen.push({label,command});
        if (command.op === heldOp) {
            heldOp = null;
            return new Promise(resolve => { pending = {command,resolve}; });
        }
        return Promise.resolve(reply(command));
    }});
    const connection = {id:'v',vessel_id:'vessel',name:'Computer',client:makeClient('old'),voyages:[]};
    voyageSettings(root,{connections:new Map([['v',connection]])},{current:() => selection,select:() => {},apply:() => {},draft:async(vessel,workspace) => {origin={key:'draft-key',vessel,workspace,target:{type:'new_chat',workspace}};},captureDraft:() => origin});

    field('new-voyage').click();
    await until(() => pending?.command.op === 'capabilities');
    assert.match(field('edit-status').textContent,/Loading workspaces/);
    const lost = pending; pending = null;
    const oldClient = connection.client;
    connection.client = null;
    oldClient.socket.dispatchEvent(new Event('close'));
    // The old socket may close before its pending command settles.
    await until(() => /connection lost/i.test(field('edit-status').textContent));
    lost.resolve(reply(lost.command));
    await Promise.resolve();
    assert.equal(field('edit-retry').hidden,false,'Reload is available on the overview screen');
    assert.equal(seen.some(item => item.command.op === 'accounts'),false,'a stale capability reply does not start later reads');

    connection.client = makeClient('after-loss');
    field('edit-retry').click();
    await until(() => field('setup-profile-list button'));

    for (const [op,next] of [['accounts','after-accounts'],['profiles','after-profiles']]) {
        heldOp = op; pending = null; field('edit-retry').click();
        await until(() => pending?.command.op === op);
        const stale = pending; pending = null;
        connection.client = makeClient(next);
        stale.resolve(reply(stale.command));
        await until(() => /connection changed/i.test(field('edit-status').textContent));
        assert.equal(field('setup-profile-list button'),null,`${op} must not restore profiles from the old client`);
        field('edit-retry').click();
        await until(() => field('setup-profile-list button'));
    }

    heldOp = 'account_models'; pending = null;
    field('setup-profile-open').click(); field('setup-manage-open').click(); field('profile-edit').click();
    await until(() => pending?.command.op === 'account_models');
    const staleModels = pending; pending = null;
    connection.client = makeClient('after-models');
    staleModels.resolve(reply(staleModels.command));
    await until(() => /connection changed/i.test(field('edit-status').textContent));
    assert.equal(field('edit-model').options.length,0,'models from the old client are ignored');
    assert.equal(field('edit-save').disabled,true);
    field('edit-retry').click();
    await until(() => !field('setup-profiles').hidden && field('setup-profile-list button'));
    assert.match(field('edit-status').textContent,/Apply copies/,'Reload from the editor returns to usable profile choices');

    heldOp = 'account_models'; pending = null;
    field('setup-manage-open').click(); field('profile-edit').click();
    await until(() => pending?.command.op === 'account_models');
    const oldEditor = pending; pending = null;
    field('setup-back').click();
    assert.equal(field('setup-manage').hidden,false);
    oldEditor.resolve(reply(oldEditor.command));
    await Promise.resolve();
    assert.equal(field('edit-model').options.length,0,'a model response cannot populate an editor after leaving it');
    assert.match(field('edit-status').textContent,/Apply copies/,'an old editor read cannot replace the newer profile status');

    heldOp = 'accounts'; pending = null;
    field('edit-retry').click();
    await until(() => pending?.command.op === 'accounts');
    const oldLocation = pending; pending = null;
    field('setup-back').click(); field('setup-back').click(); field('setup-location-open').click();
    field('edit-workspace').value = '__custom__'; field('edit-workspace').dispatchEvent(new Event('change'));
    field('edit-workspace-path').value = '/new-folder'; field('edit-workspace-path').dispatchEvent(new Event('input'));
    field('edit-workspace-path').dispatchEvent(new Event('change'));
    await until(() => field('setup-profile-list button'));
    oldLocation.resolve(reply(oldLocation.command));
    await Promise.resolve();
    assert.equal(field('edit-workspace-path').value,'/new-folder','older accounts cannot replace a newer location');
    assert.ok(field('setup-profile-list button'),'newer location choices remain available');

    heldOp = 'profiles'; pending = null; field('edit-retry').click();
    await until(() => pending?.command.op === 'profiles');
    const oldVoyage = pending; pending = null;
    selection = {vessel:'v',session_id:'different-voyage'};
    oldVoyage.resolve(reply(oldVoyage.command));
    await until(() => /Voyage changed/.test(field('edit-status').textContent));
    assert.equal(field('setup-profile-list button'),null,'an old voyage response cannot populate this setup');
});
