import {sameAccount,profileSettings,profileSummary,matchingProfile,duplicateName,profileNameError} from '../js/execution-profiles.js';
import {uuid} from '../js/vessel-client.js';
import React,{useEffect,useRef,useState} from 'react';
import {Enrollment} from './Enrollment';
import {VesselUpdate} from './VesselUpdate';
import {accountChoices,Creation,vesselRead} from './settings';
import type {Tab,Workspace} from './workspace';

type Screen='overview'|'location'|'profiles'|'manage'|'editor'|'accounts'|'models'|'reasoning'|'enrollment'|'delete'|'usage';
const titles:Record<Screen,string>={overview:'Voyage setup',location:'Location',profiles:'Choose profile',manage:'Your profiles',editor:'Edit profile',accounts:'Choose account',models:'Choose model',reasoning:'Reasoning & service',enrollment:'Connect ChatGPT',delete:'Delete profile',usage:'Account usage'};
const expiredOAuth=(account:any)=>account?.binding.transport==='chatgpt_oauth'&&account.state==='ready'&&account.availability==='expired';
function SetupRow({label,detail,onClick,disabled=false}:{label:string;detail:string;onClick:()=>void;disabled?:boolean}){
    return <button type="button" className="setup-row" onClick={onClick} disabled={disabled}><span><strong>{label}</strong><small>{detail}</small></span><span aria-hidden="true">›</span></button>;
}
export function Settings({fleet,workspace,tab,tenant,onClose,onCreated}:{fleet:any;workspace:Workspace;tab?:Tab;tenant:string;onClose:()=>void;onCreated:(vessel:string,process:any)=>void}){
    const dialog=useRef<HTMLDialogElement>(null),heading=useRef<HTMLHeadingElement>(null),body=useRef<HTMLDivElement>(null);
    const [screen,setScreen]=useState<Screen>('overview'),[search,setSearch]=useState('');
    const history=useRef<{screen:Screen;focus:HTMLElement|null}[]>([]);
    const [vessel,setVessel]=useState(tab?.vessel||[...fleet.connections.keys()][0]||'');
    const [caps,setCaps]=useState<any>(null),[path,setPath]=useState(''),[accounts,setAccounts]=useState<any[]>([]),[account,setAccount]=useState(''),[models,setModels]=useState<any[]>([]),[model,setModel]=useState(''),[reasoning,setReasoning]=useState(''),[service,setService]=useState('');
    const [usage,setUsage]=useState<any>(null),[usageNotice,setUsageNotice]=useState('');
    const [capabilityReload,setCapabilityReload]=useState(0),[accountsReload,setAccountsReload]=useState(0);
    const [catalogue,setCatalogue]=useState<any>(null),[profileId,setProfileId]=useState(''),[editor,setEditor]=useState<any>(null),[profileName,setProfileName]=useState('');
    const [notice,setNotice]=useState(''),[busy,setBusy]=useState(false),[recovery,setRecovery]=useState(0);
    const connection=fleet.connections.get(vessel),selected=accounts.find(item=>sameAccount(item.binding,account?JSON.parse(account):null)),selectedModel=models.find(item=>item.id===model);
    const profile=catalogue?.profiles.find((item:any)=>item.id===profileId),profileAccount=accounts.find(item=>sameAccount(item.binding,profile?.account));
    const generation=useRef(0),usageGeneration=useRef(0),refreshAttempts=useRef(new Set<string>());
    const creation=useRef<Creation|null>(null),origin=useRef({incarnation:tab?.incarnation,revision:tab?.snapshot?.revision});
    const lastVessel=useRef(vessel),editorRevision=useRef<number|null>(null);
    const editorSeed=useRef<any>(null),restoreFocus=useRef<HTMLElement|null>(null);
    function navigate(next:Screen){history.current.push({screen,focus:document.activeElement as HTMLElement});setSearch('');setScreen(next);}
    function back(){if(busy)return;const previous=history.current.pop();restoreFocus.current=previous?.focus||null;setSearch('');setScreen(previous?.screen||'overview');if(screen==='editor')setEditor(null);}
    function show(next:Screen){history.current=[];setSearch('');setScreen(next);}
    useEffect(()=>{dialog.current?.showModal();return()=>dialog.current?.close();},[]);
    useEffect(()=>{body.current?.scrollTo?.(0,0);const focus=restoreFocus.current;restoreFocus.current=null;if(focus?.isConnected)focus.focus();else heading.current?.focus();},[screen]);
    useEffect(()=>{usageGeneration.current++;setUsage(null);setUsageNotice('');return()=>{usageGeneration.current++;};},[vessel,path,account,screen,connection?.client]);
    useEffect(()=>{generation.current++;return()=>{generation.current++;};},[vessel,path,connection?.client]);
    useEffect(()=>{
        let alive=true;const changedVessel=lastVessel.current!==vessel;lastVessel.current=vessel;
        retainEditor();setCaps(null);setAccounts([]);setCatalogue(null);
        if(changedVessel){setPath('');setEditor(null);}
        setNotice('Loading Vessel…');
        vesselRead(connection,'capabilities').then(async value=>{
            if(value.vessel_id!==connection.vessel_id)throw new Error('Vessel identity changed.');
            if(!value.features?.includes('execution_profiles')){if(alive){setCaps(value);setScreen('location');setNotice(`Vessel ${value.version||'version unknown'} needs an update before profile setup.`);}return;}
            if(value.scope!=='owner'&&(!value.rights?.includes('account_use')||(!tab&&!value.rights?.includes('create'))))throw new Error('This connection does not permit account use or creation.');
            let location=tab?.snapshot?.workspace||(changedVessel?'':path);
            if(tab){const process=await vesselRead(connection,'inspect',{session_id:tab.session});if(process.incarnation!==tab.incarnation)throw new Error('Voyage changed. Reopen settings.');location=process.workspace;}
            if(alive){setCaps(value);setPath(location||value.workspaces?.[0]?.path||'');setNotice('');}
        }).catch(error=>alive&&setNotice(error.message));return()=>{alive=false;};
    },[vessel,connection?.client,capabilityReload]);
    useEffect(()=>{setEditor(null);setProfileId('');},[path,vessel]);
    useEffect(()=>{
        let alive=true;setCatalogue(null);
        if(path.startsWith('/'))vesselRead(connection,'profiles',{workspace:path}).then(value=>{if(alive){setCatalogue(value);setProfileId(current=>value.profiles.some((item:any)=>item.id===current)?current:matchingProfile(value.profiles,tab?.snapshot?.inference)?.id||value.default_profile_id||value.profiles[0]?.id||'');}}).catch(error=>alive&&setNotice(error.message));
        return()=>{alive=false;};
    },[path,vessel,connection?.client,recovery]);
    useEffect(()=>{
        let alive=true;setAccounts([]);setModels([]);if(!path.startsWith('/'))return;
        vesselRead(connection,'accounts',{workspace:path,transport:null}).then(value=>{if(alive){setAccounts(accountChoices(value));}}).catch(error=>alive&&setNotice(error.message));return()=>{alive=false;};
    },[path,vessel,connection?.client,recovery,accountsReload]);
    // Model discovery is editor-only. A seed is captured before refreshing accounts,
    // so navigation and sign-in completion cannot silently reset an unsaved model.
    useEffect(()=>{
        let alive=true;setModels([]);setModel('');if(!editor||!selected?.ready)return;
        setNotice('Loading models…');
        vesselRead(connection,'account_models',{workspace:path,account:selected.binding}).then(value=>{
            if(!alive)return;if(!sameAccount(value.account,selected.binding))throw new Error('Account identity changed.');
            const seed=editorSeed.current||editor,same=sameAccount(selected.binding,seed?.account);
            setModels(value.models||[]);setModel((same?value.models?.find((item:any)=>item.id===seed?.model)?.id:null)||value.models?.find((item:any)=>item.is_default)?.id||value.models?.[0]?.id||'');
            setReasoning(same?seed?.reasoning_effort||'':'');setService(same?seed?.service_tier||'':'');setNotice('');
        }).catch(error=>alive&&setNotice(error.message));return()=>{alive=false;};
    },[account,accounts,editor]);
    const pending=()=>{try{creation.current??=new Creation(localStorage,tenant);return {records:creation.current.pending(),error:false};}catch{return {records:[],error:true};}};
    const recoveryState=pending(),records=recoveryState.records;
    function retainEditor(){if(!editor||!selected||!model)return;editorSeed.current={...editor,account:selected?.binding,model,reasoning_effort:reasoning,service_tier:service};}
    async function save(){
        if(screen==='editor')return saveProfile();if(screen!=='overview'||busy||!caps||!profile)return;setBusy(true);
        try{
            const settings=profileSettings(profile);
            if(tab){if(['running','starting','cancelling'].includes(tab.snapshot?.run?.state))throw new Error('Wait for the current run to finish before changing inference settings.');if(tab.incarnation!==origin.current.incarnation||tab.snapshot?.revision!==origin.current.revision)throw new Error('Voyage changed since review. Reopen settings.');if(!await workspace.act(tab.key,'set_account_inference',settings))throw new Error('Settings were not confirmed. Check the voyage receipt before retrying.');onClose();}
            else {creation.current??=new Creation(localStorage,tenant);const process=await creation.current.start(connection,path,settings);onCreated(vessel,process);onClose();}
        }catch(error){setNotice(error instanceof Error?error.message:'Settings unavailable.');}finally{setBusy(false);}
    }
    function editProfile(mode:string){
        const value=mode==='new'?{id:uuid(),name:'',account:accounts.find(item=>item.ready)?.binding}:structuredClone(profile);if(!value)return;
        if(mode==='duplicate'){value.id=uuid();value.name=duplicateName(value.name);}
        editorRevision.current=catalogue?.revision;editorSeed.current=value;setEditor(value);setProfileName(value.name);setAccount(value.account?JSON.stringify(value.account):'');setNotice('');navigate('editor');
    }
    async function mutate(op:string,fields:any){
        if(busy||!catalogue?.can_manage)return;setBusy(true);const epoch=generation.current;
        try{const value=await vesselRead(connection,op,{command_id:uuid(),workspace:path,expected_revision:catalogue.revision,...fields});if(epoch!==generation.current)return;setCatalogue(value);setProfileId(fields.profile?.id||value.profiles.find((item:any)=>item.id===profileId)?.id||value.default_profile_id||value.profiles[0]?.id||'');setEditor(null);show('manage');setNotice('Profiles saved. Existing voyages retain their settings.');}
        catch(error){if(epoch===generation.current){setCatalogue(null);setEditor(null);show('manage');setNotice(`${error instanceof Error?error.message:'Change unconfirmed.'} Reload profiles before retrying; the change may have completed.`);}}finally{setBusy(false);}
    }
    function saveProfile(){if(!editor)return;if(catalogue?.revision!==editorRevision.current){setNotice('Saved profiles changed while you were editing. Cancel this edit and review the current profile before saving.');return;}const nameError=profileNameError(profileName);if(nameError){setNotice(nameError);return;}if(!selected?.ready||!selectedModel){setNotice('Choose an available account and model.');return;}return mutate('save_profile',{profile:{id:editor.id,name:profileName.trim(),account:selected.binding,model,reasoning_effort:reasoning||null,service_tier:service||null},make_default:false});}
    async function refreshSignIn(item:any){
        const key=JSON.stringify([vessel,path,item.binding]);if(busy||!expiredOAuth(item)||refreshAttempts.current.has(key))return;
        refreshAttempts.current.add(key);setBusy(true);setNotice(`Refreshing sign-in for ${item.label}…`);retainEditor();
        const epoch=generation.current,binding=structuredClone(item.binding);let observation:any=null,confirmed=false;
        try{
            try{observation=await vesselRead(connection,'account_usage',{workspace:path,account:binding,refresh:true});confirmed=sameAccount(observation?.account,binding);}catch{/* Never replay an uncertain refresh. Reload the catalogue once. */}
            const value=await vesselRead(connection,'accounts',{workspace:path,transport:null});if(epoch!==generation.current)return;
            const choices=accountChoices(value).map((choice:any)=>sameAccount(choice.binding,binding)&&(!confirmed||observation?.refresh_status==='sign_in_required')?{...choice,ready:false,availability:confirmed?'sign_in_required':'refresh_unconfirmed'}:choice),current=choices.find((choice:any)=>sameAccount(choice.binding,binding));setAccounts(choices);
            setNotice(!confirmed?'Sign-in refresh could not be confirmed. Reload accounts to check its status; do not repeat the refresh.':observation?.refresh_status==='sign_in_required'?'Sign-in is required. Reauthenticate this account on its Vessel with vessel auth accounts reauthenticate.':current?.ready?'Sign-in refreshed. This account is available.':current?.availability==='refresh_pending_or_uncertain'?'Sign-in refresh is pending or uncertain. Recover this account on its Vessel before retrying.':'This account is still unavailable. Check its sign-in on the Vessel.');
        }catch{if(epoch===generation.current)setNotice('Refresh was attempted, but account status could not be reloaded. Reload accounts to check; do not repeat the refresh.');}finally{setBusy(false);}
    }
    async function refreshUsage(){
        if(busy||!selected?.ready)return;const epoch=++usageGeneration.current;setBusy(true);setUsageNotice('Loading usage…');
        try{const value=await vesselRead(connection,'account_usage',{workspace:path,account:selected.binding,refresh:true});if(epoch!==usageGeneration.current)return;if(!sameAccount(value.account,selected.binding))throw new Error('Account usage identity changed.');setUsage(value);setUsageNotice('');}
        catch(error){if(epoch===usageGeneration.current)setUsageNotice(error instanceof Error?error.message:'Usage unavailable.');}finally{setBusy(false);}
    }
    const matches=(text:string)=>text.toLocaleLowerCase().includes(search.toLocaleLowerCase());
    const reasoningStops=['',...new Set<string>([...(selectedModel?.reasoning_efforts||[]),...(reasoning?[reasoning]:[])])];
    const refreshButton=(item:any)=>expiredOAuth(item)&&<button type="button" disabled={busy||refreshAttempts.current.has(JSON.stringify([vessel,path,item.binding]))} onClick={()=>void refreshSignIn(item)}>Refresh sign-in</button>;
    const canSave=!busy&&!recoveryState.error&&caps&&(screen==='editor'?selected?.ready&&selectedModel:profile&&profileAccount?.ready)&&(!!tab||path.startsWith('/'));
    return <dialog ref={dialog} className="settings-dialog profile-setup" aria-labelledby="profile-setup-title" onCancel={event=>{if(busy)event.preventDefault();else onClose();}}>
        <form onSubmit={event=>{event.preventDefault();void save();}}>
            <header>{screen!=='overview'&&<button type="button" disabled={busy} aria-label="Back" onClick={back}>←</button>}<h2 id="profile-setup-title" ref={heading} tabIndex={-1}>{screen==='overview'?(tab?'Voyage setup':'New voyage'):titles[screen]}</h2><button type="button" disabled={busy} aria-label="Close settings" onClick={onClose}>×</button></header>
            <div ref={body} className="setup-body" aria-busy={busy}>
                {screen==='overview'&&<>
                    <p>{tab?'Choose a saved profile for the next run.':'Choose where your voyage runs and the profile it uses.'}</p>
                    <SetupRow label="Location" detail={[connection?.name,path].filter(Boolean).join(' · ')||'Choose a Vessel and workspace'} disabled={busy} onClick={()=>navigate('location')}/>
                    <SetupRow label="Profile" detail={profile?`${profile.name} · ${profile.model}`:'Choose a profile'} disabled={busy||!catalogue} onClick={()=>navigate('profiles')}/>
                    {profile&&<p className="setup-summary">{profile.reasoning_effort||'Provider default reasoning'} · {profile.service_tier||'Default service tier'}<br/>{profileAccount?.label||'Account unavailable'}</p>}
                    {profileAccount&&!profileAccount.ready&&<div className="setup-warning"><p>This profile’s account is {profileAccount.availability?.replaceAll('_',' ')||'unavailable'}.</p>{refreshButton(profileAccount)}</div>}
                </>}
                {screen==='location'&&<>
                    <label>Vessel<select disabled={!!tab||busy} value={vessel} onChange={event=>setVessel(event.target.value)}>{[...fleet.connections.values()].map((item:any)=><option key={item.id} value={item.id}>{item.name}{!item.client?' · offline':''}</option>)}</select></label>
                    <label>Workspace{caps?.scope==='owner'&&!tab?<input value={path} onChange={event=>setPath(event.target.value)} placeholder="Existing absolute folder on this Vessel" disabled={busy}/>:<select disabled={!!tab||busy} value={path} onChange={event=>setPath(event.target.value)}>{tab?<option value={path}>{path}</option>:(caps?.workspaces||[]).map((item:any)=><option key={item.path} value={item.path}>{item.name} · {item.path}</option>)}</select>}</label>
                    <p>{tab?'An existing voyage keeps its Vessel and workspace.':'Use an existing folder on the Vessel.'}</p>
                    {caps&&<VesselUpdate connection={connection} caps={caps} tenant={tenant} onResume={()=>setCapabilityReload(value=>value+1)}/>}
                </>}
                {(screen==='profiles'||screen==='manage')&&<>
                    <label className="setup-search">Search profiles<input type="search" value={search} onChange={event=>setSearch(event.target.value)}/></label>
                    <div className="setup-choices">{(catalogue?.profiles||[]).filter((item:any)=>matches(`${item.name} ${item.model}`)).map((item:any)=><button type="button" className="setup-choice" aria-pressed={profileId===item.id} disabled={busy} key={item.id} onClick={()=>{setProfileId(item.id);if(screen==='profiles')back();}}><strong>{item.name}{item.id===catalogue.default_profile_id&&<span className="setup-badge">Default</span>}</strong><small>{profileSummary(item,accounts)}</small></button>)}</div>
                    {!catalogue?.profiles?.some((item:any)=>matches(`${item.name} ${item.model}`))&&<p>No profiles found.</p>}
                    {screen==='profiles'&&catalogue?.can_manage&&<button type="button" disabled={busy} onClick={()=>navigate('manage')}>Manage profiles</button>}
                    {screen==='manage'&&catalogue?.can_manage&&<><div className="setup-actions"><button type="button" disabled={busy} onClick={()=>editProfile('new')}>Create profile</button><button type="button" disabled={busy||!profile} onClick={()=>editProfile('edit')}>Edit</button><button type="button" disabled={busy||!profile} onClick={()=>editProfile('duplicate')}>Duplicate</button><button type="button" disabled={busy||!profile||profile.id===catalogue.default_profile_id} onClick={()=>void mutate('set_default_profile',{profile_id:profile.id})}>Make default</button><button type="button" disabled={busy||!profile} onClick={()=>navigate('delete')}>Delete</button></div><p>Profile changes affect future selections. Existing voyages keep their settings.</p></>}
                    {profileAccount&&!profileAccount.ready&&<div className="setup-warning"><p>The selected profile’s account is unavailable.</p>{refreshButton(profileAccount)}</div>}
                    <button type="button" disabled={busy} onClick={()=>setRecovery(value=>value+1)}>Reload profiles</button>
                </>}
                {screen==='editor'&&editor&&<>
                    <label>Profile name<input maxLength={80} value={profileName} disabled={busy} onChange={event=>setProfileName(event.target.value)}/></label>
                    <SetupRow label="Provider account" detail={selected?.label||'Choose an account'} disabled={busy} onClick={()=>navigate('accounts')}/>
                    <SetupRow label="Model" detail={selectedModel?.display_name||model||'Choose a model'} disabled={busy||!selected?.ready} onClick={()=>navigate('models')}/>
                    <SetupRow label="Reasoning & service" detail={`${reasoning||'Provider default'} · ${service||'Default tier'}`} disabled={busy||!selectedModel} onClick={()=>navigate('reasoning')}/>
                    <button type="button" disabled={busy||!selected?.ready} onClick={()=>navigate('usage')}>Account usage</button>
                </>}
                {screen==='accounts'&&<>
                    <label className="setup-search">Search accounts<input type="search" value={search} onChange={event=>setSearch(event.target.value)}/></label>
                    <div className="setup-choices">{accounts.filter(item=>matches(item.label)).map(item=><div key={JSON.stringify(item.binding)} className="setup-account"><button type="button" className="setup-choice" disabled={busy||!item.ready} aria-pressed={sameAccount(selected?.binding,item.binding)} onClick={()=>{retainEditor();setAccount(JSON.stringify(item.binding));back();}}><strong>{item.label}</strong><small>{item.ready?'Available':item.availability?.replaceAll('_',' ')||'Unavailable'}</small></button>{refreshButton(item)}</div>)}</div>
                    {!accounts.some(item=>matches(item.label))&&<p>No accounts found.</p>}
                    <div className="setup-actions"><button type="button" disabled={busy} onClick={()=>navigate('enrollment')}>Add ChatGPT account</button><button type="button" disabled={busy} onClick={()=>{retainEditor();setAccountsReload(value=>value+1);}}>Reload accounts</button></div>
                </>}
                {screen==='models'&&<>
                    <label className="setup-search">Search models<input type="search" value={search} onChange={event=>setSearch(event.target.value)}/></label>
                    <div className="setup-choices">{models.filter(item=>matches(`${item.display_name||''} ${item.id}`)).map(item=><button type="button" key={item.id} className="setup-choice" disabled={busy} aria-pressed={model===item.id} onClick={()=>{if(model!==item.id){setModel(item.id);setReasoning('');setService('');}back();}}><strong>{item.display_name||item.id}</strong><small>{item.id}{item.is_default?' · Default':''}</small></button>)}</div>
                    {!models.some(item=>matches(`${item.display_name||''} ${item.id}`))&&<p>No models found.</p>}
                </>}
                {screen==='reasoning'&&<>
                    <p>These settings belong to this profile. Save the profile when you’re done.</p>
                    <label>Reasoning <output>{reasoning||'Provider default'}</output><input className="setup-range" type="range" min="0" max={reasoningStops.length-1} step="1" value={Math.max(0,reasoningStops.indexOf(reasoning))} aria-valuetext={reasoning||'Provider default'} disabled={busy||reasoningStops.length<2} onChange={event=>setReasoning(reasoningStops[Number(event.target.value)])}/></label>
                    <div className="setup-range-labels"><span>Default</span><span>{reasoningStops.at(-1)||'Default'}</span></div>
                    <label>Service tier<select value={service} disabled={busy} onChange={event=>setService(event.target.value)}><option value="">Provider default</option>{[...new Set<string>([...(selectedModel?.service_tiers||[]),...(service?[service]:[])])].map(value=><option key={value}>{value}</option>)}</select></label>
                </>}
                {screen==='usage'&&<>
                    <p>{selected?.label}</p>
                    {(usage?.snapshot?.windows||[]).filter((window:any)=>Number.isFinite(window.used_percent)).map((window:any,index:number)=><div className="setup-usage" key={index}><strong>{String(window.kind).replaceAll('_',' ')}</strong><span>{Math.round(window.used_percent*10)/10}% used</span><progress max="100" value={Math.max(0,Math.min(100,window.used_percent))} aria-label={`${window.kind} usage`}/>{window.resets_at&&<small>Resets {new Date(window.resets_at*1000).toLocaleString()}</small>}</div>)}
                    <p role="status">{usageNotice||(!usage?.snapshot?.windows?.length?'No usage observation loaded. This is not zero usage.':`Status: ${String(usage.refresh_status||'observed').replaceAll('_',' ')}`)}</p>
                    <button type="button" disabled={busy||!selected?.ready} onClick={()=>void refreshUsage()}>Refresh usage</button>
                </>}
                {screen==='enrollment'&&connection&&path&&<Enrollment connection={connection} workspace={path} tenant={tenant} autoOpen onRefreshed={()=>{retainEditor();setAccountsReload(value=>value+1);back();}}/>}
                {screen==='delete'&&<><p>Delete <strong>{profile?.name}</strong>? Existing voyages keep their current settings.</p><button type="button" className="setup-danger" disabled={busy||!profile} onClick={()=>void mutate('delete_profile',{profile_id:profile.id})}>Delete profile</button></>}
            </div>
            <footer><p role="status">{recoveryState.error?'Recovery storage is unavailable. Do not clear it or repeat uncertain creation.':notice}</p>
                {records.map(record=><button type="button" key={record.key} disabled={busy} onClick={async()=>{setBusy(true);try{const c=fleet.connections.get(record.vessel);const process=await creation.current!.reconcile(c,record);if(process){onCreated(c.id,process);onClose();}else setNotice('Creation was not admitted. You can review and try again.');}catch(error){setNotice(error instanceof Error?error.message:'Receipt unavailable.');}finally{setBusy(false);}}}>Check creation · {record.vessel}</button>)}
                <div className="setup-footer-actions">{screen==='overview'?<><button type="button" disabled={busy} onClick={onClose}>Cancel</button><button className="primary-action" disabled={!canSave}>{busy?'Working…':tab?'Apply profile':'Create voyage'}</button></>:screen==='editor'?<><button type="button" disabled={busy} onClick={back}>Cancel profile edit</button><button className="primary-action" disabled={!canSave}>{busy?'Working…':'Save profile'}</button></>:<button type="button" disabled={busy} onClick={back}>{screen==='reasoning'||screen==='location'?'Done':'Back'}</button>}</div>
            </footer>
        </form>
    </dialog>;
}
