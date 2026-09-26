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

function fixture(t, {firstHasProfile = false} = {}) {
    const compiled = mkdtempSync(join(tmpdir(),'helm-location-setup-'));
    t.after(() => rmSync(compiled,{recursive:true,force:true}));
    const rendered = spawnSync('php',['-r',`require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); echo view('livewire.console',['vessels'=>collect(),'tenantId'=>'location-test'])->render();`],{cwd:new URL('..',import.meta.url),encoding:'utf8',env:{...process.env,VIEW_COMPILED_PATH:compiled}});
    assert.equal(rendered.status,0,rendered.stderr);
    const dom = new JSDOM(rendered.stdout,{url:'https://console.example'});
    t.after(() => dom.window.close());
    for (const key of ['window','document','localStorage','Event']) globalThis[key] = dom.window[key];
    const root = document.querySelector('#helm-client'), field = id => root.querySelector(`#${id}`);
    const account = vessel => ({account_id:`${vessel}-account`,connection_id:`${vessel}-provider`,identity_generation:1,connection_revision:1,transport:'openai_responses'});
    const profile = vessel => ({id:`${vessel}-profile`,name:`${vessel} profile`,account:account(vessel),model:`${vessel}-model`,reasoning_effort:null,service_tier:null});
    const catalogues = {
        tax:{revision:1,can_manage:true,default_profile_id:firstHasProfile ? 'tax-profile' : null,profiles:firstHasProfile ? [profile('tax')] : []},
        win:{revision:1,can_manage:true,default_profile_id:'win-profile',profiles:[profile('win')]},
    };
    const seen = [], held = new Map();
    const clients = new Map(['tax','win'].map(vessel => [vessel,{socket:new dom.window.EventTarget(),exchange(envelope) {
        const command = envelope.command;
        seen.push({vessel,command});
        const result = () => {
            if (command.op === 'capabilities') return {vessel_id:`${vessel}-identity`,scope:'owner',workspaces:[{name:vessel,path:vessel === 'tax' ? '/tax-axis' : '/home/psi'}]};
            if (command.op === 'accounts') return {accounts:[{id:account(vessel).account_id,connection_id:account(vessel).connection_id,identity_generation:1,state:'ready',availability:'available',label:`${vessel} account`}],connections:[{id:account(vessel).connection_id,revision:1,transports:['openai_responses'],label:`${vessel} provider`}]};
            if (command.op === 'profiles') return structuredClone(catalogues[vessel]);
            if (command.op === 'account_models') return {account:command.account,models:[{id:`${vessel}-model`,is_default:true}]};
            if (command.op === 'save_profile') {
                assert.equal(command.expected_revision,catalogues[vessel].revision);
                catalogues[vessel].profiles = catalogues[vessel].profiles.map(item => item.id === command.profile.id ? structuredClone(command.profile) : item);
                catalogues[vessel].revision++;
                return structuredClone(catalogues[vessel]);
            }
            throw new Error(`Unexpected ${vessel} ${command.op}`);
        };
        if (held.has(`${vessel}:${command.op}`)) return new Promise(resolve => held.set(`${vessel}:${command.op}`,{command,resolve:() => resolve({protocol:1,error:null,outcome_unknown:false,result:result()})}));
        return Promise.resolve({protocol:1,error:null,outcome_unknown:false,result:result()});
    }}]));
    const fleet = {connections:new Map(['tax','win'].map(vessel => [vessel,{id:vessel,vessel_id:`${vessel}-identity`,name:vessel,client:clients.get(vessel),voyages:[]}]))};
    let selection = {vessel:'tax',session_id:null,incarnation:null};
    const select = (vessel,session_id) => { selection = {...selection,vessel,session_id}; };
    const composition = composer(root,{fleet,current:() => selection,select,notice:message => assert.fail(message)});
    const captureDraft = () => {
        const origin = composition.capture();
        return origin?.key && composition.active ? {...origin,workspace:composition.active.document.target.workspace,target:structuredClone(composition.active.document.target)} : null;
    };
    const settings = voyageSettings(root,fleet,{current:() => selection,select,apply:() => assert.fail('Existing voyage inference was changed'),draft:(vessel,workspace) => composition.newChat(vessel,workspace),captureDraft,prepared:() => {}});
    const hold = (vessel,op) => held.set(`${vessel}:${op}`,null);
    const release = (vessel,op) => { const pending = held.get(`${vessel}:${op}`); assert.ok(pending,`${vessel} ${op} was not pending`); held.delete(`${vessel}:${op}`); pending.resolve(); };
    return {field,seen,settings,composition,captureDraft,hold,release,selection:() => selection,choose:select};
}

test('changing Vessel during new voyage setup loads editor models and retains the message through Use profile and Done', async t => {
    const f = fixture(t);
    f.field('new-voyage').click();
    await until(() => f.seen.some(item => item.vessel === 'tax' && item.command.op === 'profiles'));
    f.field('prompt').value = 'Keep this unsent message';
    f.field('prompt').dispatchEvent(new Event('input'));
    const key = f.captureDraft().key;

    f.field('setup-location-open').click();
    f.field('edit-vessel').value = 'win';
    f.field('edit-vessel').dispatchEvent(new Event('change'));
    await until(() => f.captureDraft()?.vessel === 'win' && f.field('setup-profile-list button'));
    assert.equal(f.selection().vessel,'win');
    assert.equal(f.captureDraft().key,key);
    assert.equal(f.captureDraft().workspace,'/home/psi');
    assert.equal(f.field('prompt').value,'Keep this unsent message');

    f.field('setup-back').click();
    f.field('setup-profile-open').click();
    f.field('setup-manage-open').click();
    f.field('profile-edit').click();
    await until(() => f.field('edit-model').value === 'win-model');
    assert.doesNotMatch(f.field('edit-status').textContent,/Voyage changed/);
    f.field('edit-profile-name').value = 'Win profile edited';
    f.field('edit-save').click();
    await until(() => !f.field('setup-manage').hidden && f.field('edit-status').textContent.includes('Profiles saved'));
    f.field('setup-back').click();
    assert.equal(f.field('setup-profiles').hidden,false);
    f.field('edit-save').click();
    await until(() => !f.field('setup-overview').hidden && f.settings.configuration()?.vessel === 'win');
    f.field('setup-done').click();

    assert.equal(f.field('setup-dialog').querySelector('dialog').hasAttribute('open'),false);
    assert.equal(f.field('prompt').value,'Keep this unsent message');
    assert.equal(f.settings.configuration().workspace,'/home/psi');
    assert.equal(f.settings.configuration().profileName,'Win profile edited');
    assert.deepEqual(f.seen.filter(item => ['start_account','set_account_inference'].includes(item.command.op)),[]);
    assert.ok(f.seen.some(item => item.vessel === 'win' && item.command.op === 'account_models'));
    assert.ok(f.seen.some(item => item.vessel === 'win' && item.command.op === 'save_profile'));
    assert.equal(f.seen.some(item => item.vessel === 'tax' && ['account_models','save_profile'].includes(item.command.op)),false);
});

test('an external draft change invalidates a pending model response after a Vessel handoff', async t => {
    const f = fixture(t);
    f.field('new-voyage').click();
    await until(() => f.seen.some(item => item.vessel === 'tax' && item.command.op === 'profiles'));
    f.field('setup-location-open').click();
    f.field('edit-vessel').value = 'win';
    f.field('edit-vessel').dispatchEvent(new Event('change'));
    await until(() => f.captureDraft()?.vessel === 'win' && f.field('setup-profile-list button'));
    f.hold('win','account_models');
    f.field('setup-back').click();
    f.field('setup-profile-open').click();
    f.field('setup-manage-open').click();
    f.field('profile-edit').click();
    await until(() => f.seen.some(item => item.vessel === 'win' && item.command.op === 'account_models'));
    const key = f.captureDraft().key;
    const opening = f.captureDraft().opening;

    await f.composition.newChat('tax','/tax-axis');
    assert.equal(f.captureDraft().key,key,'the composer reuses its draft key');
    assert.notEqual(f.captureDraft().opening,opening);
    f.release('win','account_models');
    await until(() => /Voyage changed/.test(f.field('edit-status').textContent));

    assert.equal(f.field('edit-model').options.length,0);
    assert.equal(f.field('edit-save').disabled,true);
    assert.equal(f.seen.some(item => item.command.op === 'save_profile'),false);
});

test('Use profile moves an already configured draft to the newly selected Vessel', async t => {
    const f = fixture(t,{firstHasProfile:true});
    f.field('new-voyage').click();
    await until(() => f.settings.configuration()?.vessel === 'tax');
    f.field('prompt').value = 'Keep the plan';
    f.field('prompt').dispatchEvent(new Event('input'));
    const key = f.captureDraft().key;

    f.field('setup-location-open').click();
    f.field('edit-vessel').value = 'win';
    f.field('edit-vessel').dispatchEvent(new Event('change'));
    await until(() => f.seen.some(item => item.vessel === 'win' && item.command.op === 'profiles') && f.field('setup-profile-list button'));
    assert.equal(f.selection().vessel,'tax','the existing draft stays put until Use profile');
    f.field('setup-back').click();
    assert.equal(f.field('setup-done').disabled,true,'the old profile needs review at the new location');
    f.field('setup-profile-open').click();
    f.field('edit-save').click();
    await until(() => f.settings.configuration()?.vessel === 'win' && !f.field('setup-overview').hidden);
    f.field('setup-done').click();

    assert.equal(f.selection().vessel,'win');
    assert.equal(f.captureDraft().key,key);
    assert.equal(f.captureDraft().workspace,'/home/psi');
    assert.equal(f.field('prompt').value,'Keep the plan');
    assert.equal(f.field('setup-dialog').querySelector('dialog').hasAttribute('open'),false);
    assert.deepEqual(f.seen.filter(item => ['start_account','set_account_inference'].includes(item.command.op)),[]);
});

test('an old Vessel response cannot replace choices after selecting another location', async t => {
    const f = fixture(t,{firstHasProfile:true});
    f.field('new-voyage').click();
    await until(() => f.settings.configuration()?.vessel === 'tax');
    f.hold('win','accounts');
    f.field('setup-location-open').click();
    f.field('edit-vessel').value = 'win';
    f.field('edit-vessel').dispatchEvent(new Event('change'));
    await until(() => f.seen.some(item => item.vessel === 'win' && item.command.op === 'accounts'));
    f.field('edit-vessel').value = 'tax';
    f.field('edit-vessel').dispatchEvent(new Event('change'));
    await until(() => f.field('edit-workspace').value === '/tax-axis' && f.field('setup-profile-list button'));
    f.release('win','accounts');
    await Promise.resolve();

    assert.equal(f.field('edit-vessel').value,'tax');
    assert.equal(f.field('edit-workspace').value,'/tax-axis');
    assert.equal(f.field('setup-profile-list button .setup-choice-title').textContent,'tax profile · Default');
    assert.equal(f.seen.some(item => item.vessel === 'win' && item.command.op === 'profiles'),false);
});

test('Use profile and Done reject an external voyage selection while Setup is open', async t => {
    const f = fixture(t,{firstHasProfile:true});
    f.field('new-voyage').click();
    await until(() => f.settings.configuration()?.vessel === 'tax');
    f.field('setup-profile-open').click();
    f.choose('tax','other-voyage');
    await f.composition.select();

    f.field('edit-save').click();
    assert.match(f.field('edit-status').textContent,/Voyage changed/);
    f.field('setup-back').click();
    f.field('setup-done').click();

    assert.match(f.field('edit-status').textContent,/Voyage changed/);
    assert.equal(f.field('setup-dialog').querySelector('dialog').hasAttribute('open'),true);
    assert.deepEqual(f.seen.filter(item => ['start_account','set_account_inference'].includes(item.command.op)),[]);
});
