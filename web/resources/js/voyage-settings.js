import {sameAccount, profileSettings, profileSummary, matchingProfile, duplicateName, profileNameError} from './execution-profiles.js';
import {accountEnrollment} from './account-enrollment.js';
import {setReasoning, reasoningValue} from './inference-controls.js';
import {request, uuid} from './vessel-client.js';

const same = (a, b) => a && b && ['account_id','connection_id','identity_generation','connection_revision','transport'].every(k => a[k] === b[k]);
const clean = value => String(value ?? '').replace(/[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/g, '');
async function read(client, op, fields = {}) {
    const response = await client.exchange(request(op, fields));
    if (response.protocol !== 1 || response.error != null || response.outcome_unknown !== false) throw new Error('The Vessel could not confirm this request. Check its connection, then reload choices.');
    return response.result;
}

export function voyageSettings(root, fleet, {current, select, apply, draft, created, captureDraft, resumeDraft = () => null, unavailable = () => {}, prepared = () => {}}) {
    const raw = id => root.querySelector(`#${id}`);
    const $ = id => raw(id === 'voyage-settings-form' ? 'edit-form' : id.replace(/^settings-/, 'edit-'));
    const creating = () => !target?.session_id;
    let openingDraft;
    let catalogue = {revision:0,profiles:[],can_manage:false}, editingProfile = null;
    let editSection = 'model', usageVersion = 0, enrolledAccount = null;
    const configurations = new Map();
    let starting = false;
    let target, connection, choices = [], models = [], defaults = {}, version = 0, saving = false;
    const enrollment = accountEnrollment(root, {context: () => ({connection, workspace: workspace()}), refreshed: async account => { enrolledAccount = account; await loadAccounts(); }});
    const prefix = `helm-web:creation:${root.dataset.tenantId}:`;
    const status = message => { $('settings-status').textContent = clean(message); };
    function options(id, items, selected = null) {
        const field = $(id);
        if (field.matches('ui-slider')) { setReasoning(field, items.length ? items : [{value:'',label:'Provider default'}], selected); return; }
        const radio = field.matches('ui-radio-group');
        const custom = field.matches('ui-select'), container = custom ? field.querySelector('ui-options') : field;
        if (custom) container.querySelectorAll('[data-settings-option]').forEach(option => option.remove());
        else container.replaceChildren();
        container.append(...items.map(item => {
            const option = $(radio ? 'flux-service-option' : custom ? 'flux-search-option' : 'flux-option').content.firstElementChild.cloneNode(true);
            option.setAttribute('data-settings-option','');
            (radio ? option.querySelector('ui-radio') || option : option).setAttribute('value', item.value);
            (option.querySelector('[data-option-label]') || option).textContent = clean(item.label);
            option.toggleAttribute('disabled', Boolean(item.disabled));
            return option;
        }));
        const value = selected != null && items.some(item => item.value === selected && !item.disabled) ? selected : items.find(item => !item.disabled)?.value ?? '';
        field.value = value;
        field.disabled = !items.some(item => !item.disabled);
    }
    const workspace = () => creating() && $('settings-workspace').value === '__custom__' ? raw('edit-workspace-path').value.trim() : $('settings-workspace').value;
    function workspaceChanged() {
        raw('edit-custom-workspace').hidden = !creating() || $('settings-workspace').value !== '__custom__';
        return loadAccounts();
    }
    const choice = () => choices[Number($('settings-account').value)];
    const model = () => models.find(m => m.id === $('settings-model').value);
    const key = (c, command) => `${prefix}${c.id}:${c.vessel_id}:${command.command_id}`;
    function reset() {
        enrollment.hide();
        choices = []; models = []; defaults = {};
        for (const id of ['settings-account','settings-model','settings-reasoning','settings-service']) options(id, []);
        $('settings-save').disabled = true;
    }
    function still(mine, c, client) { return mine === version && connection === c && c.client === client && current().session_id === target?.session_id && current().vessel === target?.vessel; }
    async function loadVessel() {
        if (saving) return;
        const mine = ++version; reset(); options('settings-workspace', []);
        raw('edit-custom-workspace').hidden = true; raw('edit-workspace-path').value = ''; raw('edit-workspace-path').disabled = false;
        connection = fleet.connections.get($('settings-vessel').value);
        const c = connection, client = c?.client;
        if (!client) { unavailable('This Vessel is offline. Reconnect it to load workspaces and accounts.'); return status('This Vessel is offline. Reconnect it to load workspaces and accounts.'); }
        status('Loading workspaces…');
        try {
            const caps = await read(client, 'capabilities');
            if (!still(mine,c,client)) return;
            if (caps.vessel_id !== c.vessel_id) throw new Error('Vessel identity changed. Reconnect before continuing.');
            const required = creating() ? ['create','account_use'] : ['account_use'];
            if (caps.scope !== 'owner' && required.some(right => !caps.rights?.includes(right))) throw new Error('This older connection has limited access. Reconnect using the full-access setup instructions.');
            if (creating()) {
                for (let i=0; i<localStorage.length; i++) if (localStorage.key(i)?.startsWith(`${prefix}${c.id}:${c.vessel_id}:`)) throw new Error('This Vessel has an unconfirmed creation. Close this form and use Check creation in the sidebar before creating another voyage.');
            }
            let workspaces = caps.workspaces || [];
            if (!creating()) {
                const process = await read(client,'inspect',{session_id:target.session_id});
                if (!still(mine,c,client)) return;
                if (process.session_id !== target.session_id || process.incarnation !== target.incarnation) throw new Error('Voyage changed. Close this form and reopen its settings.');
                workspaces = [{name:process.workspace,path:process.workspace}];
            }
            const items = workspaces.map(w => ({value:w.path,label:w.name === w.path ? w.path : `${w.name} · ${w.path}`}));
            if (creating() && caps.scope === 'owner') items.push({value:'__custom__',label:'Another folder…'});
            const origin = creating() ? captureDraft?.() : null;
            const preferred = origin?.vessel === c.id && origin.target?.type === 'new_chat' ? origin.workspace : null;
            options('settings-workspace',items,preferred && items.some(item=>item.value===preferred) ? preferred : preferred && caps.scope === 'owner' ? '__custom__' : null);
            if (preferred && !items.some(item=>item.value===preferred) && caps.scope === 'owner') $('settings-workspace-path').value=preferred;
            $('settings-workspace').disabled = !creating() || !items.length;
            if (!items.length) return status('No workspaces are available. Reconnect using the full-access setup instructions.');
            await workspaceChanged();
        } catch (error) { if (still(mine,c,client)) { status(error.message); if (creating() && !configuration()) unavailable(error.message); } }
    }
    async function loadAccounts() {
        if (saving) return;
        const mine = ++version; reset();
        catalogue = {revision:0,profiles:[],can_manage:false}; renderProfiles();
        const c = connection, client = c?.client, selectedWorkspace = workspace();
        if (!client || !selectedWorkspace) return status('Choose a connected Vessel and workspace.');
        if (!selectedWorkspace.startsWith('/')) return status('Enter an absolute folder path on this Vessel.');
        status('Loading provider accounts…');
        try {
            const accountCatalogue = await read(client,'accounts',{workspace:selectedWorkspace,transport:null});
            if (!still(mine,c,client)) return;
            for (const account of accountCatalogue.accounts || []) {
                const provider = accountCatalogue.connections?.find(item => item.id === account.connection_id);
                for (const transport of provider?.transports || []) {
                    const binding = {account_id:account.id,connection_id:provider.id,identity_generation:account.identity_generation,connection_revision:provider.revision,transport};
                    const ready = account.state === 'ready' && account.availability === 'available';
                    choices.push({binding,ready,label:`${account.label} · ${provider.label} · ${transport.replaceAll('_',' ')}${same(binding,accountCatalogue.default_account) ? ' · Default' : ''}${ready ? '' : ` · ${account.availability.replaceAll('_',' ')}`}`});
                }
            }
            const reviewed = configuration();
            if (creating() && reviewed?.vessel === c.id && reviewed.workspace === selectedWorkspace) defaults = reviewed.settings;
            const preferred = choices.find(c => c.binding.account_id === enrolledAccount)?.binding || (!creating() ? target.inference?.account : defaults.account);
            enrolledAccount = null;
            options('settings-account', choices.map((c,i) => ({value:String(i),label:c.label,disabled:!c.ready})), String(choices.findIndex(c => same(c.binding,preferred))));
            if (editSection === 'account') loadUsage(false);
            await loadProfiles();
        } catch (error) { if (still(mine,c,client)) { status(error.message); if (creating() && !configuration()) unavailable(error.message); } }
    }
    const selectedProfile = () => catalogue.profiles.find(profile => profile.id === raw('edit-profile').value);
    function showProfileEditor(show) {
        for (const section of ['account','model','reasoning','service','profile-name']) raw(`edit-${section}-section`).hidden = !show;
        raw('edit-profiles-section').hidden = show;
        raw('edit-save').textContent = show ? 'Save profile' : 'Apply';
    }
    function renderProfiles(preferred) {
        const seed = configuration()?.settings || target?.inference;
        raw('edit-profile-actions').querySelectorAll('button').forEach(button=>button.disabled=false);
        options('settings-profile', catalogue.profiles.map(profile => ({value:profile.id,label:`${profile.name}${profile.id === catalogue.default_profile_id ? ' · Default' : ''}`})), preferred || matchingProfile(catalogue.profiles,seed)?.id || catalogue.default_profile_id);
        raw('edit-profile-actions').hidden = !catalogue.can_manage;
        showProfileEditor(false); editingProfile = null;
        profileChanged();
    }
    function profileChanged() {
        const profile = selectedProfile();
        raw('edit-profile-summary').textContent = profileSummary(profile,choices);
        raw('edit-save').disabled = !profile || !choices.some(item => item.ready && sameAccount(item.binding,profile.account));
        for (const action of ['edit','duplicate','default','delete']) raw(`profile-${action}`).disabled = !profile;
        status(profile ? 'Apply copies these settings to this voyage. Profile edits never change existing voyages.' : 'Create a profile to get started.');
        if (creating() && profile && !configuration() && !raw('edit-save').disabled) {
            const origin = captureDraft?.(), c = connection, mine = version;
            Promise.resolve(origin?.workspace ? null : draft(c.id,workspace())).then(() => {
                if (!still(mine,c,c.client)) return;
                const active = captureDraft?.();
                if (!active || active.vessel !== c.id || active.workspace !== workspace()) return;
                configurations.set(active.key,{vessel:c.id,vessel_id:c.vessel_id,workspace:workspace(),profileName:profile.name,accountLabel:choices.find(item=>sameAccount(item.binding,profile.account))?.label,settings:profileSettings(profile),reasoning_efforts:[]});
                prepared();
            });
        }
    }
    async function loadProfiles() {
        const mine = version, c = connection, client = c.client;
        const result = await read(client,'profiles',{workspace:workspace()});
        if (!still(mine,c,client)) return;
        catalogue = result; renderProfiles();
    }
    async function editProfile(mode) {
        if (saving || !catalogue.can_manage) return;
        const profile = mode === 'new' ? null : selectedProfile();
        editingProfile = profile ? structuredClone(profile) : {id:uuid(),name:'',account:choice()?.binding};
        if (mode === 'duplicate') { editingProfile.id=uuid(); editingProfile.name = duplicateName(editingProfile.name); }
        raw('edit-profile-name').value = editingProfile.name;
        for (const id of ['edit-profile-name','profile-cancel-edit','edit-account','edit-model','edit-reasoning','edit-service']) raw(id).disabled=false;
        options('settings-account',choices.map((item,index)=>({value:String(index),label:item.label,disabled:!item.ready})),String(choices.findIndex(item=>sameAccount(item.binding,editingProfile.account))));
        showProfileEditor(true); await loadModels();
    }
    async function mutateProfiles(op, fields) {
        saving = true;
        const fieldsBefore = [...raw('edit-form').querySelectorAll('select,input,ui-select,ui-slider,ui-radio-group,button')].map(field=>[field,field.disabled]);
        const restoreFields = () => fieldsBefore.forEach(([field,disabled])=>field.disabled=disabled);
        fieldsBefore.forEach(([field])=>field.disabled=true);
        raw('edit-save').disabled = true;
        raw('edit-profile-actions').querySelectorAll('button').forEach(button=>button.disabled=true);
        try {
            catalogue = await read(connection.client,op,{command_id:uuid(),workspace:workspace(),expected_revision:catalogue.revision,...fields});
            restoreFields(); renderProfiles(fields.profile?.id);
            status('Profiles saved. Existing voyage settings are unchanged.');
        } catch (error) { restoreFields(); catalogue = {revision:0,profiles:[],can_manage:false}; editingProfile=null; renderProfiles(); status(`${error.message} Reload profiles before retrying; the previous change may have completed.`); }
        finally { saving=false; raw('edit-retry').disabled=false; raw('edit-close').disabled=false; raw('profile-new').disabled=false; }
    }
    async function saveProfile() {
        const selected = choice(), selectedModel = model(), name = raw('edit-profile-name').value.trim();
        const nameError = profileNameError(name);
        if (nameError) return status(nameError);
        if (!selected?.ready || !selectedModel) return status('Choose an available account and model.');
        return mutateProfiles('save_profile',{profile:{id:editingProfile.id,name,account:selected.binding,model:selectedModel.id,reasoning_effort:reasoningValue(raw('edit-reasoning')) || null,service_tier:raw('edit-service').value || null},make_default:false});
    }
    raw('edit-profile').addEventListener('change',profileChanged);
    for (const mode of ['new','edit','duplicate']) raw(`profile-${mode}`).addEventListener('click',()=>editProfile(mode));
    raw('profile-cancel-edit').addEventListener('click',()=>renderProfiles());
    raw('profile-default').addEventListener('click',()=>{if(!saving && selectedProfile()) mutateProfiles('set_default_profile',{profile_id:selectedProfile().id});});
    raw('profile-delete').addEventListener('click',()=>{if(!saving && selectedProfile()) mutateProfiles('delete_profile',{profile_id:selectedProfile().id});});
    async function loadModels() {
        if (saving) return;
        const mine = ++version, c = connection, client = c?.client, selected = choice();
        models = []; $('settings-save').disabled = true;
        for (const id of ['settings-model','settings-reasoning','settings-service']) options(id,[]);
        if (!client || !selected?.ready) return status('Choose an available provider account.');
        status('Loading models for this account…');
        try {
            const result = await read(client,'account_models',{workspace:workspace(),account:selected.binding});
            if (!still(mine,c,client)) return;
            if (!same(result.account,selected.binding) || !Array.isArray(result.models)) throw new Error('Account model list changed. Reload choices.');
            models = result.models;
            const seed = editingProfile || (!creating() ? target.inference : defaults);
            const preferred = same(seed?.account,selected.binding) ? seed?.model : models.find(m => m.is_default)?.id;
            options('settings-model',models.map(m => ({value:m.id,label:m.display_name && m.display_name !== m.id ? `${m.display_name} · ${m.id}` : m.id})),preferred);
            updateModel();
        } catch (error) { if (still(mine,c,client)) { status(error.message); if (creating() && !configuration()) unavailable(error.message); } }
    }
    function updateModel() {
        const selected = model();
        const seed = editingProfile || (!creating() ? target.inference : defaults);
        const preserve = same(seed?.account,choice()?.binding) && seed?.model === selected?.id;
        for (const [id, values, original] of [['settings-reasoning',selected?.reasoning_efforts,seed?.reasoning_effort],['settings-service',selected?.service_tiers,seed?.service_tier]]) {
            const items = [{value:'',label:'Provider default'},...(values || []).map(v => ({value:v,label:v}))];
            // Existing explicit values remain visible; changing account/model deliberately resets them.
            if (preserve && original && !items.some(v => v.value === original)) items.push({value:original,label:`${original} · current`});
            options(id,items,preserve ? original || '' : '');
        }
        $('settings-save').disabled = !selected;
        if (creating() && selected && !configuration() && !editingProfile) {
            const origin = captureDraft?.(), mine = version, c = connection;
            if (origin?.vessel === connection.id && origin.target?.type === 'new_chat' && !origin.workspace) {
                draft(connection.id,workspace()).then(() => {
                    if (!still(mine,c,c.client) || current().session_id || captureDraft()?.key !== origin.key) return;
                    const active = captureDraft();
                    configurations.set(active.key,{vessel:connection.id,vessel_id:connection.vessel_id,workspace:workspace(),accountLabel:clean(choice().label),settings:{account:choice().binding,model:selected.id,reasoning_effort:reasoningValue($('settings-reasoning')) || null,service_tier:$('settings-service').value || null},reasoning_efforts:selected.reasoning_efforts || []});
                    prepared();
                });
            }
        }

        status(selected ? creating() ? 'Your first Send creates the voyage and sends the message.' : 'Changes apply to the next run. Changing account or model resets its options to provider defaults.' : 'This account returned no models. Reload choices or choose another account.');
    }
    function open(edit) {
        enrollment.hide();
        if (saving) return;
        root.querySelector('[data-flux-sidebar-on-mobile]:not([data-flux-sidebar-collapsed-mobile]) [data-flux-sidebar-collapse] button')?.click();
        ++usageVersion;
        target = current(); openingDraft = captureDraft?.()?.key;
        $('settings-retry').disabled = false;
        $('settings-close').disabled = false;
        $('settings-title').textContent = edit ? 'Profiles' : 'New voyage';
        editingProfile = null; showProfileEditor(false); raw('edit-location-section').hidden = editSection !== 'location';
        $('settings-save').textContent = 'Apply';
        $('settings-description').textContent = 'Choose a profile. Changes apply to the next run; editing or deleting a profile does not change existing voyages.';
        options('settings-vessel',[...fleet.connections.values()].map(c => ({value:c.id,label:`${c.name}${c.client ? '' : ' · offline'}`})),target.vessel);
        $('settings-vessel').disabled = !creating();
        return loadVessel();
    }
    async function accept(c, command, process, storageKey, origin) {
        if (process?.session_id !== command.session_id || process.workspace !== command.workspace || typeof process.incarnation !== 'string') throw new Error('Creation identity could not be confirmed. Use Check creation.');
        if (!c.voyages.some(v => v.session_id === process.session_id)) c.voyages.unshift(process);
        c.lastCatalogue = 0;
        const retained = await created?.(c.id,process.session_id,command.workspace,origin);
        if (!retained) select(c.id,process.session_id,process.name || 'New voyage');
        localStorage.removeItem(storageKey);
        raw('edit-close').click();
        renderPending();
    }
    async function save(event) {
        event.preventDefault();
        if (saving || $('settings-save').disabled) return;
        if (current().vessel !== target?.vessel || current().session_id !== target?.session_id || (creating() && captureDraft?.()?.key !== openingDraft)) return status('Voyage changed. Reopen settings.');
        if (editingProfile) return saveProfile();
        const c = connection, client = c?.client, profile = selectedProfile();
        if (!client || !profile) return status('Choose a profile before continuing.');
        const settings = profileSettings(profile), selected = choices.find(item => sameAccount(item.binding,profile.account)), selectedModel = {reasoning_efforts:[]};
        if (!selected?.ready) return status('This profile’s provider account is unavailable. Edit the profile or choose another.');
        saving = true; ++version;
        status(!creating() ? 'Applying account and model settings…' : 'Preparing new chat…');
        for (const field of $('voyage-settings-form').querySelectorAll('select,input,ui-select,ui-slider,ui-radio-group,button')) field.disabled = true;
        let recorded = false;
        try {
            if (!creating()) {
                if (!await apply(target,settings)) throw new Error('Settings were not confirmed. Close this form to review the voyage status before trying again.');
                $('settings-close').disabled = false; $('settings-close').click();
            } else {
                const path = workspace();
                let origin = captureDraft?.();
                if (origin?.target?.type === 'new_chat' && (origin.vessel !== c.id || origin.workspace !== path)) { await draft?.(c.id,path); origin = captureDraft?.(); }
                if (!origin || origin.target?.type !== 'new_chat') {
                    await draft?.(c.id,path);
                    origin = captureDraft?.();
                }
                if (!origin || origin.vessel !== c.id || origin.workspace !== path || origin.target?.session_id) throw new Error('The new-chat composer could not be prepared.');
                configurations.set(origin.key,{vessel:c.id,vessel_id:c.vessel_id,workspace:path,profileName:profile.name,accountLabel:clean(selected.label),settings:structuredClone(settings),reasoning_efforts:selectedModel.reasoning_efforts || []});
                raw('edit-close').disabled = false;
                raw('edit-close').click();
                status('New chat ready. Write your message, then Send.');
                prepared();

            }
        } catch (error) { status(error.message); }
        finally {
            saving = false;
            $('settings-close').disabled = false;
            // After dispatch only explicit resolution can settle a creation.
            $('settings-retry').disabled = recorded;
            renderPending();
            if (!recorded) { $('settings-retry').disabled = false; }
        }
    }
    // Configuration is in memory and explicitly reviewed before creating a chat.
    function configuration(origin = captureDraft?.()) {
        const value = origin && configurations.get(origin.key), c = value && fleet.connections.get(value.vessel);
        return value && c?.client && c.vessel_id === value.vessel_id && origin.vessel === value.vessel && origin.workspace === value.workspace && !origin.target?.session_id ? value : null;
    }
    async function startDraft(origin = captureDraft?.()) {
        if (starting) throw new Error('New chat creation is already in progress.');
        const config = configuration(origin);
        if (!config) throw new Error('Choose a profile for this new chat before sending.');
        const c = fleet.connections.get(config.vessel);
        // An unresolved creation must never be replaced, even after a reload or review.
        for (let i=0;i<localStorage.length;i++) {
            const storageKey=localStorage.key(i);
            if (!storageKey?.startsWith(prefix)) continue;
            const record=JSON.parse(localStorage.getItem(storageKey));
            if (record.origin?.key === origin.key) throw new Error('Creation is unconfirmed. Use Check creation before sending.');
        }
        const command=request('start_account',{command_id:uuid(),session_id:uuid(),workspace:config.workspace,...config.settings}).command;
        const storageKey=key(c,command);
        starting=true;
        try {
            localStorage.setItem(storageKey,JSON.stringify({vessel:c.id,vessel_id:c.vessel_id,command,origin}));
            const result=await c.client.exchange({protocol:1,command});
            if (result.protocol !== 1 || result.outcome_unknown !== false) throw new Error('Creation is unconfirmed. Use Check creation before sending.');
            if (result.error != null) { localStorage.removeItem(storageKey); throw new Error('Creation was rejected. Your draft is retained; review settings before retrying.'); }
            await accept(c,command,result.result,storageKey,origin);
            configurations.delete(origin.key);
            return result.result;
        } finally { starting=false; renderPending(); }
    }
    function renderPending() {
        $('pending-creations').replaceChildren();
        try {
            for (let i=0; i<localStorage.length; i++) {
                const storageKey = localStorage.key(i); if (!storageKey?.startsWith(prefix)) continue;
                const record = JSON.parse(localStorage.getItem(storageKey));
                const c = fleet.connections.get(record.vessel);
                if (!c || record.vessel_id !== c.vessel_id || key(c,record.command) !== storageKey) continue;
                const node = $('flux-action').content.firstElementChild.cloneNode(true);
                node.querySelector('[data-label]').textContent = clean(`Check creation · ${c.name}`); node.disabled = !c.client;
                node.addEventListener('click',async () => {
                    node.disabled = true;
                    try {
                        const result = await read(c.client,'resolve_start_account',{...record.command,op:'resolve_start_account'});
                        if (result.command_id !== record.command.command_id || result.session_id !== record.command.session_id) throw new Error('Creation identity changed.');
                        if (result.status === 'created') await accept(c,record.command,result.process,storageKey,record.origin);
                        else if (result.status === 'not_admitted') { localStorage.removeItem(storageKey); renderPending(); }
                        else throw new Error('Creation remains unconfirmed. Check again after reconnecting.');
                    } catch (error) { node.querySelector('[data-label]').textContent = clean(`${c.name} · ${error.message}`); node.disabled = !c.client; }
                });
                $('pending-creations').append(node);
            }
        } catch { status('Browser recovery storage is unavailable. Creation requires working local storage.'); }
    }
    for (const id of ['account-popover','edit-popover','service-popover']) raw(id)?.addEventListener('toggle', event => { if (event.newState === 'closed') enrollment.hide(); });
    for (const prefix of ['edit']) {
        raw(`${prefix}-enroll`).addEventListener('click', () => {
            raw(`${prefix}-enrollment-host`).append(raw('enrollment-panel'));
            enrollment.open();
        });
        raw(`${prefix}-enrollment-host`).addEventListener('toggle', event => {
            if (event.newState === 'closed' && raw(`${prefix}-enrollment-host`).contains(raw('enrollment-panel'))) enrollment.hide();
        });
        raw(`${prefix}-close`).addEventListener('click', () => enrollment.hide());
    }
    async function newVoyage() {
        if (saving || starting) return;
        const retained = resumeDraft();
        const c = fleet.connections.get(retained?.vessel || current().vessel) || [...fleet.connections.values()].find(c => c.client) || [...fleet.connections.values()][0];
        if (!c) return unavailable('Connect a Vessel before starting a new voyage.');
        await draft?.(c.id, retained?.workspace || '');
        prepared();
        editSection = 'location';
        open(false);
    }
    raw('new-voyage').addEventListener('click',() => newVoyage().catch(error => unavailable(error.message)));
    function openSection(section, host, event) {
        enrollment.hide();
        if (saving) return;
        editSection = section;
        raw(host).append(raw('edit-form'));
        raw('edit-location-section').hidden = section !== 'location';
        open(true);
    }
    raw('change-location').addEventListener('click',event => openSection('location','location-popover',event));
    raw('change-inference').addEventListener('click',event => openSection('model','edit-popover',event));
    raw('change-account').addEventListener('click',event => openSection('account','account-popover',event));
    raw('change-service').addEventListener('click',event => openSection('service','service-popover',event));
    async function loadUsage(refresh) {
        const mine = ++usageVersion, c = connection, client = c?.client, selected = choice(), workspace = $('settings-workspace').value;
        const output = raw('account-usage'), button = raw('account-usage-refresh');
        output.textContent = refresh ? 'Refreshing usage…' : 'Reading cached usage…'; button.disabled = true;
        if (!client || !selected?.ready) { output.textContent = 'Usage unavailable for this account.'; return; }
        try {
            const observation = await read(client,'account_usage',{workspace,account:selected.binding,refresh});
            if (mine !== usageVersion || editSection !== 'account' || c !== connection || client !== c.client || !same(selected.binding,choice()?.binding)) return;
            if (!same(observation.account,selected.binding)) throw new Error('Account usage identity changed. Reopen the account picker.');
            const snapshot = observation.snapshot;
            const lines = [`Status: ${String(observation.refresh_status || 'unavailable').replaceAll('_',' ')}`];
            if (snapshot) {
                lines.push(`Observed: ${new Date(snapshot.fetched_at * 1000).toLocaleString()}`);
                for (const window of snapshot.windows || []) {
                    if (!Number.isFinite(window.used_percent)) continue;
                    lines.push(`${window.kind}: ${window.used_percent}% used${window.resets_at ? ` · resets ${new Date(window.resets_at * 1000).toLocaleString()}` : ''}`);
                }
            } else lines.push('No usage observation available. This is not zero usage.');
            output.textContent = lines.map(clean).join('\n');
        } catch (error) { if (mine === usageVersion) output.textContent = clean(error.message); }
        finally { if (mine === usageVersion) button.disabled = false; }
    }
    raw('account-usage-refresh').addEventListener('click',() => loadUsage(true));
    for (const prefix of ['edit']) {
        raw(`${prefix}-vessel`).addEventListener('change',loadVessel);
        raw(`${prefix}-workspace`).addEventListener('change',workspaceChanged);
        raw(`${prefix}-account`).addEventListener('change',() => { loadModels(); if (editSection === 'account') loadUsage(false); });
        raw(`${prefix}-model`).addEventListener('change',updateModel);
        raw(`${prefix}-retry`).addEventListener('click',loadVessel);
    }
    raw('edit-workspace-path').addEventListener('input',() => { ++version; reset(); });
    raw('edit-workspace-path').addEventListener('change',loadAccounts);
    raw('edit-draft')?.addEventListener('click',async()=>{try{if(!connection || !workspace())throw new Error('Choose a Vessel and workspace.');await draft?.(connection.id,workspace());raw('edit-close').click();}catch(error){status(error.message);}});

    raw('edit-save').addEventListener('click',save);
    raw('edit-close').addEventListener('click', () => raw('edit-form').closest('[data-flux-popover]')?.hidePopover?.());
    window.addEventListener('storage',renderPending);
    return {renderPending: () => { enrollment.changed(); renderPending(); }, configuration, startDraft, review: () => raw('change-inference').click(), setReasoning(value) { const config = configuration(); if (!config) return false; config.settings.reasoning_effort = value || null; prepared(); return true; }};
}
