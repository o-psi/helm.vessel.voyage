import test from 'node:test';
import assert from 'node:assert/strict';
import React,{act} from 'react';
import {JSDOM} from 'jsdom';
import {Settings} from '../resources/react/Settings';

const binding={account_id:'a',connection_id:'p',identity_generation:1,connection_revision:1,transport:'chatgpt_oauth'};
async function mount({availability='available',usageReply,modelReply,existing=false}: {availability?:string;usageReply?:(command:any)=>Promise<any>;modelReply?:(command:any)=>Promise<any>;existing?:boolean}={}){
    const dom=new JSDOM('<div id="root"></div>',{url:'https://helm.test'});
    Object.assign(globalThis,{window:dom.window,document:dom.window.document,localStorage:dom.window.localStorage,IS_REACT_ACT_ENVIRONMENT:true});
    dom.window.HTMLDialogElement.prototype.showModal=function(){};dom.window.HTMLDialogElement.prototype.close=function(){};
    const {createRoot}=await import('react-dom/client');
    const commands:any[]=[],actions:any[]=[];let closed=0;
    const catalogue={revision:1,can_manage:true,default_profile_id:'first',profiles:[{id:'first',name:'Everyday',account:{...binding},model:'m',reasoning_effort:'high',service_tier:'flex'}]};
    const connection:any={id:'c',name:'Vessel',vessel_id:'v',voyages:[],client:{async exchange({command}:any){
        commands.push(command);let result:any;
        switch(command.op){
            case 'capabilities':result={vessel_id:'v',scope:'owner',features:['execution_profiles'],workspaces:[{path:'/work',name:'Work'}]};break;
            case 'inspect':result={incarnation:'i',workspace:'/work'};break;
            case 'accounts':result={accounts:[{id:'a',connection_id:'p',identity_generation:1,label:'Work',state:'ready',availability},{id:'b',connection_id:'p',identity_generation:1,label:'Personal',state:'ready',availability:'available'}],connections:[{id:'p',revision:1,label:'ChatGPT',transports:['chatgpt_oauth']}]};break;
            case 'profiles':result=structuredClone(catalogue);break;
            case 'account_models':result=modelReply?await modelReply(command):{account:command.account,models:[{id:'m',display_name:'Everyday model',is_default:true,reasoning_efforts:['low','high'],service_tiers:['flex']},{id:'other',display_name:'Other model',reasoning_efforts:['low'],service_tiers:[]}]};break;
            case 'account_usage':result=usageReply?await usageReply(command):{account:command.account};availability='available';break;
            case 'save_profile':result={...catalogue,revision:2,profiles:[command.profile]};break;
            default:throw new Error(command.op);
        }
        return {protocol:1,outcome_unknown:false,error:null,result};
    }}};
    const root=createRoot(document.getElementById('root')!);
    const props={fleet:{connections:new Map([['c',connection]])},workspace:{async act(...args:any[]){actions.push(args);return true;}} as any,tenant:'test',tab:existing?{key:'k',vessel:'c',session:'s',incarnation:'i',snapshot:{revision:4,run:{state:'idle'},workspace:'/work',inference:catalogue.profiles[0]}} as any:undefined,onClose(){closed++;},onCreated(){assert.fail('No creation expected');}};
    await act(async()=>root.render(React.createElement(Settings,props)));
    function button(label:string){const match=[...document.querySelectorAll<HTMLButtonElement>('button')].find(item=>(item.getAttribute('aria-label')||item.textContent||'')===label||(item.classList.contains('setup-row')&&item.querySelector('strong')?.textContent===label));assert.ok(match,label);return match;}
    async function click(label:string){await act(async()=>button(label).click());}
    async function fill(label:string,value:string){const input=[...document.querySelectorAll('label')].find(item=>item.textContent?.startsWith(label))?.querySelector('input');assert.ok(input,label);await act(async()=>{Object.getOwnPropertyDescriptor(dom.window.HTMLInputElement.prototype,'value')!.set!.call(input,value);input.dispatchEvent(new dom.window.Event('input',{bubbles:true}));});}
    return {commands,actions,button,click,fill,async renew(profilesChanged=false){if(profilesChanged)catalogue.revision++;connection.client={...connection.client};await act(async()=>root.render(React.createElement(Settings,{...props})));},closed:()=>closed,text:()=>document.body.textContent!,async dispose(){await act(async()=>root.unmount());dom.window.close();}};
}

test('profile navigation preserves unsaved name/model and keeps the current step independent',async()=>{
    const view=await mount();try{
        assert.equal(document.querySelector('select'),null,'overview is compact');
        await view.click('Profile');await view.click('Manage profiles');await view.click('Edit');
        await view.fill('Profile name','My draft');
        await view.click('Model');await view.fill('Search models','Other');
        assert.equal(document.querySelectorAll('.setup-choice').length,1,'model search filters the actual choices');
        await act(async()=>document.querySelector<HTMLButtonElement>('.setup-choice')!.click());
        await view.click('Provider account');await view.click('Back');
        assert.equal(document.querySelector<HTMLInputElement>('input')!.value,'My draft');
        assert.match(view.text(),/Other model/);
        await view.click('Reasoning & service');
        assert.equal(document.querySelector<HTMLInputElement>('input[type=range]')!.max,'1','reasoning follows selected model support');
        await view.click('Done');await view.click('Save profile');
        const saved=view.commands.find(command=>command.op==='save_profile');
        assert.equal(saved.profile.name,'My draft');assert.equal(saved.profile.model,'other');assert.equal(saved.profile.reasoning_effort,null);assert.equal(saved.profile.service_tier,null);
        assert.equal(view.actions.length,0,'saving a profile never changes the voyage');
    }finally{await view.dispose();}
});

test('existing voyage requires explicit overview apply after choosing a profile',async()=>{
    const view=await mount({existing:true});try{
        await view.click('Profile');await act(async()=>document.querySelector<HTMLButtonElement>('.setup-choice')!.click());
        assert.equal(view.actions.length,0);await view.click('Apply profile');
        assert.equal(view.actions.length,1);assert.deepEqual(view.actions[0],['k','set_account_inference',{account:binding,model:'m',reasoning_effort:'high',service_tier:'flex'}]);assert.equal(view.closed(),1);
    }finally{await view.dispose();}
});

test('expired sign-in refresh uses the exact binding, reloads accounts and enables the saved profile',async()=>{
    const view=await mount({availability:'expired',usageReply:async command=>({account:command.account,refresh_status:'fresh'})});try{
        assert.equal(view.button('Create voyage').disabled,true);await view.click('Refresh sign-in');
        assert.deepEqual(view.commands.find(command=>command.op==='account_usage').account,binding);
        assert.equal(view.commands.filter(command=>command.op==='account_usage').length,1);
        assert.equal(view.commands.filter(command=>command.op==='accounts').length,2);
        assert.match(view.text(),/Sign-in refreshed/);assert.equal(view.button('Create voyage').disabled,false);
    }finally{await view.dispose();}
});

test('lost sign-in refresh is never replayed by repeat clicks or account reload',async()=>{
    const view=await mount({availability:'expired',usageReply:async()=>{throw new Error('lost reply');}});try{
        await view.click('Refresh sign-in');assert.match(view.text(),/could not be confirmed/);
        assert.equal([...document.querySelectorAll('button')].some(button=>button.textContent==='Refresh sign-in'),false,'an unconfirmed refresh cannot be retried');
        await view.click('Profile');await view.click('Reload profiles');
        assert.equal(view.button('Refresh sign-in').disabled,true);assert.equal(view.commands.filter(command=>command.op==='account_usage').length,1);
    }finally{await view.dispose();}
});

test('wrong identity in refresh response is not presented as a successful refresh',async()=>{
    const view=await mount({availability:'expired',usageReply:async()=>({account:{...binding,identity_generation:99},refresh_status:'fresh'})});try{
        await view.click('Refresh sign-in');assert.match(view.text(),/could not be confirmed/);assert.doesNotMatch(view.text(),/Sign-in refreshed/);assert.equal(view.button('Create voyage').disabled,true,'review is required after an identity mismatch');await view.click('Profile');await view.click('Reload profiles');await view.click('Back');assert.equal(view.button('Create voyage').disabled,false,'explicit reload reviews the current catalogue');
    }finally{await view.dispose();}
});

test('late model reply from the previous account cannot replace current model choices',async()=>{
    let resolveOld!:(value:any)=>void;
    const view=await mount({modelReply:async command=>command.account.account_id==='a'?new Promise(resolve=>{resolveOld=resolve;}):{account:command.account,models:[{id:'personal-model',display_name:'Personal model',is_default:true}]}});try{
        await view.click('Profile');await view.click('Manage profiles');await view.click('Edit');await view.click('Provider account');
        await act(async()=>[...document.querySelectorAll<HTMLButtonElement>('.setup-choice')].find(item=>item.textContent?.startsWith('Personal'))!.click());
        await act(async()=>resolveOld({account:binding,models:[{id:'stale-model',display_name:'Stale model'}]}));
        await view.click('Model');assert.match(view.text(),/Personal model/);assert.doesNotMatch(view.text(),/Stale model/);
    }finally{await view.dispose();}
});


test('connection renewal while in a picker retains the unsaved profile and model',async()=>{
    const view=await mount();try{
        await view.click('Profile');await view.click('Manage profiles');await view.click('Edit');
        await view.fill('Profile name','Keep this draft');await view.click('Model');
        await act(async()=>[...document.querySelectorAll<HTMLButtonElement>('.setup-choice')].find(item=>item.textContent?.startsWith('Other model'))!.click());
        await view.click('Reasoning & service');await view.renew();await view.click('Done');
        assert.equal(document.querySelector<HTMLInputElement>('input')!.value,'Keep this draft');assert.match(view.text(),/Other model/);
        await view.click('Save profile');assert.equal(view.commands.find(command=>command.op==='save_profile').profile.model,'other');
        assert.equal(view.actions.length,0);
    }finally{await view.dispose();}
});


test('renewal cannot silently approve overwriting a newer profile revision',async()=>{
    const view=await mount();try{
        await view.click('Profile');await view.click('Manage profiles');await view.click('Edit');
        await view.fill('Profile name','Keep for review');await view.renew(true);await view.click('Save profile');
        assert.match(view.text(),/Saved profiles changed/);assert.equal(document.querySelector<HTMLInputElement>('input')!.value,'Keep for review');
        assert.equal(view.commands.some(command=>command.op==='save_profile'),false);
    }finally{await view.dispose();}
});
