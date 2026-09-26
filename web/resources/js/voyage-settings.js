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
    let usageVersion = 0, enrolledAccount = null;
    let setupScreen = 'overview', screenStack = [], focusStack = [], pickerKind = null, profileBeforePicker = null, deleteId = null;
    const configurations = new Map();
    let starting = false;
    let target, connection, choices = [], models = [], defaults = {}, version = 0, saving = false;
    const disconnectedClients = new WeakSet();
    const enrollment = accountEnrollment(root, {context: () => ({connection, workspace: workspace()}), refreshed: async account => {
        const returnToEditor = setupScreen === 'enrollment' && editingProfile
            ? {profile:structuredClone(editingProfile),name:raw('edit-profile-name').value} : null;
        enrolledAccount = account;
        await loadAccounts();
        if (returnToEditor && setupScreen === 'enrollment') {
            editingProfile = returnToEditor.profile;
            raw('edit-profile-name').value = returnToEditor.name;
            if (screenStack.at(-1) === 'editor') { screenStack.pop(); focusStack.pop(); }
            showProfileEditor(true);
            showScreen('editor', false);
            renderEditorChoices();
            if (choice()?.ready) {
                await loadModels();
                status('Account connected. Choose a model, then save the profile.');
            } else status('Account connected, but the account list could not be refreshed. Reload choices before saving.');
        }
    }});
    const prefix = `helm-web:creation:${root.dataset.tenantId}:`;
    const status = message => {
        $('settings-status').textContent = clean(message);
        if (raw('setup-overview-status')) raw('setup-overview-status').textContent = clean(message);
    };
    const setupDialog = () => raw('setup-dialog')?.querySelector('dialog');
    function showSetup() {
        if (typeof window.Flux?.modal === 'function') window.Flux.modal('voyage-setup').show();
        else setupDialog()?.setAttribute('open', '');
    }
    function closeSetup(force = false) {
        if (saving && !force) return;
        enrollment.hide();
        if (typeof window.Flux?.modal === 'function') window.Flux.modal('voyage-setup').close();
        else setupDialog()?.removeAttribute('open');
    }
    const screens = ['overview','location','profiles','manage','editor','picker','enrollment','access','delete','reasoning'];
    function showScreen(next, push = true) {
        if (push && next !== setupScreen) {
            screenStack.push(setupScreen);
            focusStack.push(document.activeElement);
        }
        setupScreen = next;
        for (const screen of screens) raw(`setup-${screen}`).hidden = screen !== next;
        raw('setup-back').hidden = next === 'overview';
        raw('setup-title').textContent = ({overview:creating() ? 'New voyage setup' : 'Voyage setup',location:'Location',profiles:'Choose profile',manage:'Manage profiles',editor:editingProfile?.name ? 'Edit profile' : 'Create profile',picker:pickerKind === 'account' ? 'Choose account' : 'Choose model',enrollment:'Connect ChatGPT',access:'Access mode',delete:'Delete profile',reasoning:'Reasoning'})[next];
        raw('setup-done').hidden = next !== 'overview';
        raw('edit-save').hidden = !['profiles','editor'].includes(next);
        raw('edit-save').textContent = next === 'editor' ? 'Save profile' : creating() ? 'Use profile' : 'Apply to next run';
        raw('edit-retry').hidden = !['overview','location','profiles','manage','editor'].includes(next);
        raw('edit-close').hidden = next !== 'overview';
        raw('edit-status').hidden = ['picker','enrollment','access','reasoning','delete'].includes(next);
        raw('setup-content').scrollTop = 0;
        renderOverview();
        raw('setup-title').focus();
    }
    function goBack() {
        if (setupScreen === 'enrollment') enrollment.hide();
        if (setupScreen === 'editor') renderProfiles(raw('edit-profile').value);
        if (setupScreen === 'profiles' && profileBeforePicker && raw('edit-profile').value !== profileBeforePicker) {
            raw('edit-profile').value = profileBeforePicker;
            profileChanged();
        }
        const previous = focusStack.pop();
        showScreen(screenStack.pop() || 'overview', false);
        if (previous?.isConnected && !previous.closest('[hidden]')) previous.focus();
    }
    function renderOverview() {
        if (!raw('setup-location-summary')) return;
        const vessel = fleet.connections.get(connection?.id || target?.vessel);
        raw('setup-location-summary').textContent = clean([vessel?.name, workspace()].filter(Boolean).join(' · ') || 'Choose a Vessel and workspace');
        const reviewed = creating() ? configuration() : null;
        const configured = reviewed && reviewed.vessel === connection?.id && reviewed.workspace === workspace() ? reviewed : null;
        const active = configured?.settings || target?.inference;
        const profile = catalogue.profiles.find(item => item.id === configured?.profileId) || matchingProfile(catalogue.profiles, active);
        raw('setup-profile-summary').textContent = clean(creating() && !configured && reviewed ? 'Choose a profile for this location' : profile ? `${profile.name} · ${profile.model}` : active?.model ? `Custom · ${active.model}` : 'Choose a profile');
        raw('setup-done').disabled = creating() && !configured;
    }
    function options(id, items, selected = null) {
        const field = $(id);
        if (field.matches('ui-slider')) { setReasoning(field, items.length ? items : [{value:'',label:'Provider default'}], selected); return; }
        const custom = field.matches('ui-select'), container = custom ? field.querySelector('ui-options') : field;
        if (custom) container.querySelectorAll('[data-settings-option]').forEach(option => option.remove());
        else container.replaceChildren();
        container.append(...items.map(item => {
            const option = $(custom ? 'flux-search-option' : 'flux-option').content.firstElementChild.cloneNode(true);
            option.setAttribute('data-settings-option','');
            option.setAttribute('value', item.value);
            (option.querySelector('[data-option-label]') || option).textContent = clean(item.label);
            if (custom) {
                option.setAttribute('label', clean(item.label));
                if (item.detail) option.setAttribute('keywords', clean(item.detail));
                const detail = option.querySelector('[data-option-detail]');
                if (detail) { detail.textContent = clean(item.detail || ''); detail.hidden = !item.detail; }
            }
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
    function sameSetup() {
        const selected = current();
        if (!creating()) return selected.session_id === target?.session_id && selected.vessel === target?.vessel && selected.incarnation === target?.incarnation;
        const active = captureDraft?.();
        if (selected.session_id) return false;
        if (!openingDraft) return !active && selected.vessel === target?.vessel;
        return active?.target?.type === 'new_chat' && active.key === openingDraft.key && active.opening === openingDraft.opening && active.vessel === selected.vessel;
    }
    async function prepareDraft(vessel, path) {
        if (!sameSetup()) throw new Error('Voyage changed. Reopen settings.');
        const before = captureDraft?.(), mine = version, c = connection, client = c?.client;
        await draft?.(vessel,path);
        const active = captureDraft?.();
        const sameLocation = mine === version && c === connection && c?.client === client && workspace() === path;
        const sameDraft = active?.key && active.target?.type === 'new_chat' && active.vessel === vessel && active.workspace === path
            && !current().session_id && current().vessel === vessel && (!before?.key || active.key === before.key)
            && (!Number.isInteger(before?.opening) || active.opening === before.opening + 1);
        if (!sameLocation || !sameDraft) throw new Error('The new-chat composer could not be prepared.');
        openingDraft = {key:active.key,opening:active.opening};
        return active;
    }
    function still(mine, c, client) { return Boolean(client) && mine === version && connection === c && c.client === client && !disconnectedClients.has(client) && sameSetup(); }
    function readStillCurrent(mine, c, client, disconnected = false) {
        if (!disconnected && still(mine,c,client)) return true;
        if (mine === version && connection === c && setupDialog()?.hasAttribute('open')) {
            if (!sameSetup()) status('Voyage changed. Close and reopen setup.');
            else if (disconnected || disconnectedClients.has(client) || c.client !== client) status(c.client && c.client !== client ? 'Vessel connection changed. Select Reload to refresh choices.' : 'Vessel connection lost. Reconnect it, then select Reload.');
        }
        return false;
    }
    async function setupRead(mine, c, client, op, fields = {}) {
        const socket = client.socket;
        const closed = () => { disconnectedClients.add(client); readStillCurrent(mine,c,client,true); };
        socket?.addEventListener('close',closed,{once:true});
        try { return await read(client,op,fields); }
        finally { socket?.removeEventListener('close',closed); }
    }
    async function loadVessel() {
        if (saving) return;
        const mine = ++version; reset(); options('settings-workspace', []);
        catalogue = {revision:0,profiles:[],can_manage:false}; renderProfiles();
        raw('edit-custom-workspace').hidden = true; raw('edit-workspace-path').value = ''; raw('edit-workspace-path').disabled = false;
        connection = fleet.connections.get($('settings-vessel').value);
        const c = connection, client = c?.client;
        if (!client) { unavailable('This Vessel is offline. Reconnect it to load workspaces and accounts.'); return status('This Vessel is offline. Reconnect it to load workspaces and accounts.'); }
        status('Loading workspaces…');
        try {
            const caps = await setupRead(mine,c,client,'capabilities');
            if (!readStillCurrent(mine,c,client)) return;
            if (caps.vessel_id !== c.vessel_id) throw new Error('Vessel identity changed. Reconnect before continuing.');
            const required = creating() ? ['create','account_use'] : ['account_use'];
            if (caps.scope !== 'owner' && required.some(right => !caps.rights?.includes(right))) throw new Error('This older connection has limited access. Reconnect using the full-access setup instructions.');
            if (creating()) {
                for (let i=0; i<localStorage.length; i++) if (localStorage.key(i)?.startsWith(`${prefix}${c.id}:${c.vessel_id}:`)) throw new Error('This Vessel has an unconfirmed creation. Close this form and use Check creation in the sidebar before creating another voyage.');
            }
            let workspaces = caps.workspaces || [];
            if (!creating()) {
                const process = await setupRead(mine,c,client,'inspect',{session_id:target.session_id});
                if (!readStillCurrent(mine,c,client)) return;
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
        } catch (error) { if (readStillCurrent(mine,c,client)) { status(error.message); if (creating() && !configuration()) unavailable(error.message); } }
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
            const accountCatalogue = await setupRead(mine,c,client,'accounts',{workspace:selectedWorkspace,transport:null});
            if (!readStillCurrent(mine,c,client)) return;
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
            options('settings-account', choices.map((c,i) => ({value:String(i),label:c.label,detail:c.ready ? 'Available' : 'Unavailable',disabled:!c.ready})), String(choices.findIndex(c => same(c.binding,preferred))));
            if (setupScreen === 'editor') loadUsage(false);
            await loadProfiles();
        } catch (error) { if (readStillCurrent(mine,c,client)) { status(error.message); if (creating() && !configuration()) unavailable(error.message); } }
    }
    const selectedProfile = () => catalogue.profiles.find(profile => profile.id === raw('edit-profile').value);
    function renderEditorChoices() {
        const account = choice(), selectedModel = model();
        raw('setup-account-value').textContent = clean(account?.label || 'Choose an account');
        raw('setup-model-value').textContent = clean(selectedModel?.display_name || selectedModel?.id || 'Choose a model');
        raw('setup-account-open').disabled = !choices.length || saving;
        raw('setup-model-open').disabled = !models.length || saving;
    }
    function openPicker(kind) {
        pickerKind = kind;
        raw('setup-picker-account').hidden = kind !== 'account';
        raw('setup-picker-model').hidden = kind !== 'model';
        showScreen('picker');
        const trigger = raw(kind === 'account' ? 'edit-account' : 'edit-model').querySelector('button, input');
        trigger?.focus();
        trigger?.click();
    }
    function showProfileEditor(show) {
        for (const section of ['account','model','reasoning','service','profile-name']) raw(`edit-${section}-section`).hidden = !show;
        raw('edit-profiles-section').hidden = show;
        raw('edit-save').textContent = show ? 'Save profile' : 'Apply';
    }
    function renderProfiles(preferred) {
        const seed = configuration()?.settings || target?.inference;
        raw('edit-profile-actions').querySelectorAll('button').forEach(button=>button.disabled=false);
        const profiles = catalogue.profiles.map(profile => {
            const available = choices.some(item => item.ready && sameAccount(item.binding,profile.account));
            return {value:profile.id,label:`${profile.name}${profile.id === catalogue.default_profile_id ? ' · Default' : ''}`,
                detail:`${profileSummary(profile,choices)}${available ? '' : ' · Account unavailable'}`,disabled:!available};
        });
        const selected = preferred || matchingProfile(catalogue.profiles,seed)?.id || catalogue.default_profile_id;
        options('settings-profile',profiles,selected);
        options('settings-manage-profile',profiles.map(profile => ({...profile,disabled:false})),raw('edit-profile').value);
        raw('edit-profile-actions').hidden = !catalogue.can_manage;
        raw('setup-manage-open').hidden = !catalogue.can_manage;
        showProfileEditor(false); editingProfile = null;
        profileChanged();
        renderOverview();
    }
    function profileChanged() {
        const profile = selectedProfile();
        raw('edit-profile-summary').textContent = profileSummary(profile,choices);
        raw('edit-save').disabled = !profile || !choices.some(item => item.ready && sameAccount(item.binding,profile.account));
        for (const action of ['edit','duplicate','default','delete']) raw(`profile-${action}`).disabled = !profile;
        status(profile ? 'Apply copies these settings to this voyage. Profile edits never change existing voyages.' : 'Create a profile to get started.');
        if (creating() && profile && !configuration() && !raw('edit-save').disabled) {
            const origin = captureDraft?.(), c = connection, client = c?.client, mine = version;
            Promise.resolve(origin?.workspace ? null : prepareDraft(c.id,workspace())).then(() => {
                if (!still(mine,c,client) || selectedProfile()?.id !== profile.id) return;
                const active = captureDraft?.();
                if (!active || active.vessel !== c.id || active.workspace !== workspace()) return;
                configurations.set(active.key,{vessel:c.id,vessel_id:c.vessel_id,workspace:workspace(),profileId:profile.id,profileName:profile.name,accountLabel:choices.find(item=>sameAccount(item.binding,profile.account))?.label,settings:profileSettings(profile),reasoning_efforts:[]});
                renderOverview(); prepared();
            }).catch(error => { if (mine === version && connection === c && setupDialog()?.hasAttribute('open')) status(error.message); });
        }
        raw('edit-manage-profile').value = raw('edit-profile').value;
        renderOverview();
    }
    async function loadProfiles() {
        const mine = version, c = connection, client = c.client;
        const result = await setupRead(mine,c,client,'profiles',{workspace:workspace()});
        if (!readStillCurrent(mine,c,client)) return;
        catalogue = result; renderProfiles();
    }
    async function editProfile(mode) {
        if (saving || !catalogue.can_manage) return;
        const profile = mode === 'new' ? null : selectedProfile();
        editingProfile = profile ? structuredClone(profile) : {id:uuid(),name:'',account:choice()?.binding};
        if (mode === 'duplicate') { editingProfile.id=uuid(); editingProfile.name = duplicateName(editingProfile.name); }
        raw('edit-profile-name').value = editingProfile.name;
        for (const id of ['edit-profile-name','profile-cancel-edit','edit-account','edit-model','edit-reasoning','edit-service']) raw(id).disabled=false;
        options('settings-account',choices.map((item,index)=>({value:String(index),label:item.label,detail:item.ready ? 'Available' : 'Unavailable',disabled:!item.ready})),String(choices.findIndex(item=>sameAccount(item.binding,editingProfile.account))));
        showProfileEditor(true); showScreen('editor'); renderEditorChoices(); await loadModels();
    }
    async function mutateProfiles(op, fields) {
        saving = true;
        const fieldsBefore = [...raw('edit-form').querySelectorAll('select,input,ui-select,ui-slider,button')].map(field=>[field,field.disabled]);
        const restoreFields = () => fieldsBefore.forEach(([field,disabled])=>field.disabled=disabled);
        fieldsBefore.forEach(([field])=>field.disabled=true);
        raw('edit-save').disabled = true;
        raw('edit-profile-actions').querySelectorAll('button').forEach(button=>button.disabled=true);
        try {
            catalogue = await read(connection.client,op,{command_id:uuid(),workspace:workspace(),expected_revision:catalogue.revision,...fields});
            restoreFields(); renderProfiles(fields.profile?.id);
            if (['editor','delete'].includes(setupScreen)) {
                if (screenStack.at(-1) === 'manage') { screenStack.pop(); focusStack.pop(); }
                showScreen('manage', false);
            }
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
    raw('edit-manage-profile').addEventListener('change',() => {
        raw('edit-profile').value = raw('edit-manage-profile').value;
        profileChanged();
    });
    for (const mode of ['new','edit','duplicate']) raw(`profile-${mode}`).addEventListener('click',()=>editProfile(mode));
    raw('profile-cancel-edit').addEventListener('click',goBack);
    raw('profile-default').addEventListener('click',()=>{if(!saving && selectedProfile()) mutateProfiles('set_default_profile',{profile_id:selectedProfile().id});});
    raw('profile-delete').addEventListener('click',()=>{
        if (saving || !selectedProfile()) return;
        deleteId = selectedProfile().id;
        raw('setup-delete-name').textContent = clean(selectedProfile().name);
        showScreen('delete');
    });
    raw('setup-delete-confirm').addEventListener('click',()=>{
        if (!saving && deleteId && catalogue.profiles.some(profile => profile.id === deleteId)) mutateProfiles('delete_profile',{profile_id:deleteId});
    });
    async function loadModels() {
        if (saving) return;
        const mine = ++version, c = connection, client = c?.client, selected = choice(), profile = editingProfile;
        models = []; $('settings-save').disabled = true;
        for (const id of ['settings-model','settings-reasoning','settings-service']) options(id,[]);
        renderEditorChoices();
        if (!client || !selected?.ready) return status('Choose an available provider account.');
        status('Loading models for this account…');
        try {
            const result = await setupRead(mine,c,client,'account_models',{workspace:workspace(),account:selected.binding});
            if (!readStillCurrent(mine,c,client) || setupScreen !== 'editor' || editingProfile !== profile) return;
            if (!same(result.account,selected.binding) || !Array.isArray(result.models)) throw new Error('Account model list changed. Reload choices.');
            models = result.models;
            const seed = editingProfile || (!creating() ? target.inference : defaults);
            const preferred = same(seed?.account,selected.binding) ? seed?.model : models.find(m => m.is_default)?.id;
            options('settings-model',models.map(m => ({value:m.id,label:m.display_name || m.id,detail:m.display_name && m.display_name !== m.id ? m.id : ''})),preferred);
            updateModel(); renderEditorChoices();
        } catch (error) { if (setupScreen === 'editor' && editingProfile === profile && readStillCurrent(mine,c,client)) { status(error.message); if (creating() && !configuration()) unavailable(error.message); } }
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
        renderEditorChoices();
        if (creating() && selected && !configuration() && !editingProfile) {
            const origin = captureDraft?.(), mine = version, c = connection, client = c?.client;
            if (origin?.vessel === connection.id && origin.target?.type === 'new_chat' && !origin.workspace) {
                prepareDraft(connection.id,workspace()).then(() => {
                    if (!still(mine,c,client) || current().session_id || captureDraft()?.key !== origin.key || model()?.id !== selected.id) return;
                    const active = captureDraft();
                    configurations.set(active.key,{vessel:connection.id,vessel_id:connection.vessel_id,workspace:workspace(),accountLabel:clean(choice().label),settings:{account:choice().binding,model:selected.id,reasoning_effort:reasoningValue($('settings-reasoning')) || null,service_tier:$('settings-service').value || null},reasoning_efforts:selected.reasoning_efforts || []});
                    prepared();
                }).catch(error => { if (mine === version && connection === c && setupDialog()?.hasAttribute('open')) status(error.message); });
            }
        }

        status(selected ? creating() ? 'Your first Send creates the voyage and sends the message.' : 'Changes apply to the next run. Changing account or model resets its options to provider defaults.' : 'This account returned no models. Reload choices or choose another account.');
    }
    function open(edit) {
        enrollment.hide();
        if (saving) return;
        root.querySelector('[data-flux-sidebar-on-mobile]:not([data-flux-sidebar-collapsed-mobile]) [data-flux-sidebar-collapse] button')?.click();
        ++usageVersion;
        target = current();
        const origin = captureDraft?.();
        openingDraft = origin ? {key:origin.key,opening:origin.opening} : null;
        $('settings-retry').disabled = false;
        $('settings-close').disabled = false;
        $('settings-title').textContent = edit ? 'Profiles' : 'New voyage';
        raw('setup-access-status').textContent = '';
        raw('reasoning-status').textContent = '';
        editingProfile = null; showProfileEditor(false);
        screenStack = []; focusStack = []; showScreen('overview', false);
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
        closeSetup();
        renderPending();
    }
    async function save(event) {
        event.preventDefault();
        if (saving || $('settings-save').disabled) return;
        if (!sameSetup()) return status('Voyage changed. Reopen settings.');
        if (editingProfile) return saveProfile();
        const c = connection, client = c?.client, profile = selectedProfile();
        if (!client || !profile) return status('Choose a profile before continuing.');
        const settings = profileSettings(profile), selected = choices.find(item => sameAccount(item.binding,profile.account)), selectedModel = {reasoning_efforts:[]};
        if (!selected?.ready) return status('This profile’s provider account is unavailable. Edit the profile or choose another.');
        saving = true; ++version;
        status(!creating() ? 'Applying account and model settings…' : 'Preparing new chat…');
        const fieldsBefore = [...$('voyage-settings-form').querySelectorAll('select,input,ui-select,ui-slider,button')].map(field => [field,field.disabled]);
        fieldsBefore.forEach(([field]) => field.disabled = true);
        let recorded = false;
        try {
            if (!creating()) {
                if (!await apply(target,settings)) throw new Error('Settings were not confirmed. Close this form to review the voyage status before trying again.');
                $('settings-close').disabled = false; closeSetup(true);
            } else {
                const path = workspace();
                let origin = captureDraft?.();
                if (origin?.target?.type === 'new_chat' && (origin.vessel !== c.id || origin.workspace !== path)) { origin = await prepareDraft(c.id,path); }
                if (!origin || origin.target?.type !== 'new_chat') {
                    origin = await prepareDraft(c.id,path);
                }
                if (!origin || origin.vessel !== c.id || origin.workspace !== path || origin.target?.session_id) throw new Error('The new-chat composer could not be prepared.');
                configurations.set(origin.key,{vessel:c.id,vessel_id:c.vessel_id,workspace:path,profileId:profile.id,profileName:profile.name,accountLabel:clean(selected.label),settings:structuredClone(settings),reasoning_efforts:selectedModel.reasoning_efforts || []});
                raw('edit-close').disabled = false;
                status('New chat ready. Write your message, then Send.');
                screenStack = []; focusStack = []; showScreen('overview', false); prepared();

            }
        } catch (error) { status(error.message); }
        finally {
            saving = false;
            fieldsBefore.forEach(([field,disabled]) => { field.disabled = disabled; });
            renderOverview();
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
        return value && c?.client && c.vessel_id === value.vessel_id && origin.vessel === value.vessel && origin.workspace === value.workspace && !origin.session_id && origin.target?.type === 'new_chat' ? value : null;
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
    raw('edit-enroll').addEventListener('click', () => {
        raw('setup-enrollment').append(raw('enrollment-panel'));
        showScreen('enrollment'); enrollment.open();
    });
    raw('enrollment-close').addEventListener('click', () => {
        if (setupScreen === 'enrollment') goBack();
    });
    async function newVoyage() {
        if (saving || starting) return;
        const retained = resumeDraft();
        const c = fleet.connections.get(retained?.vessel || current().vessel) || [...fleet.connections.values()].find(c => c.client) || [...fleet.connections.values()][0];
        if (!c) return unavailable('Connect a Vessel before starting a new voyage.');
        await draft?.(c.id, retained?.workspace || '');
        prepared();
        showSetup(); open(false);
    }
    raw('new-voyage').addEventListener('click',() => newVoyage().catch(error => unavailable(error.message)));
    raw('change-setup').addEventListener('click', () => {
        if (saving) return;
        showSetup(); open(Boolean(current().session_id));
    });
    raw('setup-close').addEventListener('click', () => closeSetup());
    raw('setup-back').addEventListener('click', goBack);
    raw('setup-done').addEventListener('click', () => {
        if (!sameSetup()) return status('Voyage changed. Reopen settings.');
        if (creating() && !configuration()) return status('Choose an available profile before continuing.');
        closeSetup(); prepared();
    });
    raw('setup-location-open').addEventListener('click', () => showScreen('location'));
    raw('setup-profile-open').addEventListener('click', () => {
        profileBeforePicker = raw('edit-profile').value;
        showScreen('profiles');
    });
    raw('setup-access-open').addEventListener('click', () => showScreen('access'));
    raw('setup-reasoning-open').addEventListener('click', () => {
        raw('change-reasoning').click();
        showScreen('reasoning');
    });
    raw('setup-manage-open').addEventListener('click', () => showScreen('manage'));
    raw('setup-manage-back').addEventListener('click', goBack);
    raw('setup-account-open').addEventListener('click', () => openPicker('account'));
    raw('setup-model-open').addEventListener('click', () => openPicker('model'));
    async function loadUsage(refresh) {
        const mine = ++usageVersion, c = connection, client = c?.client, selected = choice(), selectedWorkspace = workspace();
        const output = raw('account-usage'), button = raw('account-usage-refresh');
        output.textContent = refresh ? 'Refreshing usage…' : 'Reading cached usage…'; button.disabled = true;
        if (!client || !selected?.ready) { output.textContent = 'Usage unavailable for this account.'; return; }
        try {
            const observation = await read(client,'account_usage',{workspace:selectedWorkspace,account:selected.binding,refresh});
            if (mine !== usageVersion || setupScreen !== 'editor' || c !== connection || client !== c.client || !same(selected.binding,choice()?.binding)) return;
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
        raw(`${prefix}-account`).addEventListener('change',() => {
            if (setupScreen === 'picker' && pickerKind === 'account') goBack();
            loadModels();
            if (setupScreen === 'editor') loadUsage(false);
        });
        raw(`${prefix}-model`).addEventListener('change',() => {
            updateModel();
            if (setupScreen === 'picker' && pickerKind === 'model') goBack();
        });
        raw(`${prefix}-retry`).addEventListener('click',() => {
            if (setupScreen === 'editor') { screenStack = []; focusStack = []; showScreen('profiles', false); }
            loadVessel();
        });
    }
    raw('edit-workspace-path').addEventListener('input',() => { ++version; reset(); renderOverview(); });
    raw('edit-workspace-path').addEventListener('change',loadAccounts);
    raw('edit-draft')?.addEventListener('click',async()=>{try{if(!connection || !workspace())throw new Error('Choose a Vessel and workspace.');await prepareDraft(connection.id,workspace());raw('edit-close').click();}catch(error){status(error.message);}});

    raw('edit-save').addEventListener('click',save);
    raw('edit-close').addEventListener('click', () => closeSetup());
    window.addEventListener('storage',renderPending);
    return {renderPending: () => { enrollment.changed(); renderPending(); }, configuration, startDraft, review: () => raw('change-setup').click(), finishReasoning: () => { if (setupScreen === 'reasoning') goBack(); }, setReasoning(value) { const config = configuration(); if (!config) return false; config.settings.reasoning_effort = value || null; prepared(); return true; }};
}
