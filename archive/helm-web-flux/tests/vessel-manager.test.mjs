import test from 'node:test';
import assert from 'node:assert/strict';
import {spawnSync} from 'node:child_process';
import {mkdtempSync, rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {JSDOM} from 'jsdom';
import {openVesselManager} from '../resources/js/vessel-manager.js';

test('management links open in place; modified clicks and missing Flux retain navigation', () => {
    const d = new JSDOM('<a href="/connections"><span>Manage</span></a><div data-connections-url="https://helm.test/connections"></div>', {url:'https://helm.test/'}).window.document;
    let opened=0;
    const flux={modal(name){assert.equal(name,'manage-vessels');return {show(){opened++;}};}};
    const event=()=>({target:d.querySelector('span'),button:0,preventDefault(){this.defaultPrevented=true;}});
    const click=event(); openVesselManager(click,d,flux);
    assert.equal(opened,1); assert.equal(click.defaultPrevented,true);
    for(const extra of [{ctrlKey:true},{metaKey:true},{shiftKey:true},{altKey:true},{button:1},{defaultPrevented:true}]) {
        openVesselManager({...event(),...extra},d,flux);
    }
    openVesselManager(event(),d,null); assert.equal(opened,1);
});

for (const populated of [false,true]) test(`actual management Blade renders ${populated?'saved and pending':'empty'} state without secrets`, t => {
    const compiled=mkdtempSync(join(tmpdir(),'helm-vessel-modal-'));
    t.after(()=>rmSync(compiled,{recursive:true,force:true}));
    const code=`require 'vendor/autoload.php'; $app=require 'bootstrap/app.php'; $app->make(Illuminate\\Contracts\\Console\\Kernel::class)->bootstrap(); view()->share('errors',new Illuminate\\Support\\ViewErrorBag()); session()->put('_token','modal-test'); session()->flash('manage_vessels',true); session()->flash('status','Vessel connected.'); $connection=(object)['id'=>'saved-id','name'=>'Laptop <script>bad</script>','endpoint'=>'https://vessel.example','vessel_id'=>'vessel-id','updated_at'=>null,'credential'=>'PRIVATE-CREDENTIAL']; $pairing=(object)['id'=>'pending-id','name'=>'Pending host','request'=>'PRIVATE-INVITATION']; echo view('connections.modal',['vessels'=>collect(${populated?'[$connection]':'[]'}),'pairings'=>collect(${populated?'[$pairing]':'[]'}),'tenant'=>(object)['principal_id'=>'principal-id']])->render();`;
    const r=spawnSync('php',['-r',code],{cwd:new URL('..',import.meta.url),encoding:'utf8',env:{...process.env,VIEW_COMPILED_PATH:compiled}});
    assert.equal(r.status,0,r.stderr+r.stdout);
    const d=new JSDOM(r.stdout).window.document;
    assert.ok(d.querySelector('dialog'));
    const grid=d.querySelector('[data-vessel-grid]');
    assert.ok(grid.classList.contains('grid'));
    assert.ok(grid.classList.contains('grid-cols-1'));
    assert.ok(grid.classList.contains('sm:grid-cols-2'));
    assert.equal(grid.querySelectorAll('[data-vessel-card]').length,populated?1:0);
    if(populated) assert.ok(grid.querySelector('[data-vessel-card] form[action$="/saved-id"]'));
    else assert.ok(grid.firstElementChild.classList.contains('col-span-full'));
    assert.ok(d.querySelector('[data-vessel-list][x-show="!adding"]'));
    assert.ok(d.querySelector('[data-vessel-add][x-show="adding"][x-cloak]'));
    assert.equal(d.querySelector('[data-vessel-list] textarea'),null);
    assert.match(d.querySelector('[data-vessel-add] form').textContent,/Connect Vessel/);
    assert.doesNotMatch(d.querySelector('[data-vessel-list]').textContent,/64|Saved does not mean online|Pair a Vessel/);
    assert.match(d.querySelector('[x-init]').getAttribute('x-init'),/manage-vessels.*show/);
    assert.match(d.querySelector('[role="status"]').textContent,/Vessel connected/);
    assert.equal(d.querySelectorAll('script').length,0);
    assert.doesNotMatch(r.stdout,/PRIVATE-CREDENTIAL|PRIVATE-INVITATION/);
    for(const form of d.querySelectorAll('form')) {
        assert.equal(form.method,'post');
        assert.equal(form.querySelector('[name="_token"]').value,'modal-test');
    }
    for(const secret of d.querySelectorAll('textarea')) assert.equal(secret.value,'');
    if(populated){
        assert.match(d.body.textContent,/Laptop <script>bad<\/script>/);
        const removal=d.querySelector('form[action$="/saved-id"]');
        assert.equal(removal.querySelector('[name="_method"]').value,'DELETE');
        assert.ok(removal.querySelector('[name="confirm_disconnect"][required]'));
        assert.ok(d.querySelector('form[action$="/pending-id/retry"]'));
    } else assert.match(d.body.textContent,/No Vessels yet/);
});
