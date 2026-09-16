import {request, uuid} from './vessel-client.js';

const same = (a, b) => a && b && ['account_id','connection_id','identity_generation','connection_revision','transport'].every(k => a[k] === b[k]);
const clean = value => String(value ?? '').replace(/[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/g, '');
async function read(client, op, fields = {}) {
    const response = await client.exchange(request(op, fields));
    if (response.protocol !== 1 || response.error != null || response.outcome_unknown !== false) throw new Error('The Vessel could not confirm this request. Check its connection and access grants, then reload choices.');
    return response.result;
}

export function voyageSettings(root, fleet, {current, select, apply}) {
    const raw = id => root.querySelector(`#${id}`);
    const $ = id => raw(mode === 'edit' ? id === 'voyage-settings-form' ? 'edit-form' : id.replace(/^settings-/, 'edit-') : id);
    let mode = 'create', target, connection, choices = [], models = [], defaults = {}, version = 0, saving = false;
    const prefix = `helm-web:creation:${root.dataset.tenantId}:`;
    const status = message => { $('settings-status').textContent = clean(message); };
    function options(id, items, selected = null) {
        const field = $(id);
        const custom = field.matches('ui-select'), container = custom ? field.querySelector('ui-options') : field;
        if (custom) container.querySelectorAll('[data-settings-option]').forEach(option => option.remove());
        else container.replaceChildren();
        container.append(...items.map(item => {
            const option = $(custom ? 'flux-search-option' : 'flux-option').content.firstElementChild.cloneNode(true);
            option.setAttribute('data-settings-option','');
            option.setAttribute('value', item.value);
            (option.querySelector('[data-option-label]') || option).textContent = clean(item.label);
            option.toggleAttribute('disabled', Boolean(item.disabled));
            return option;
        }));
        const value = selected != null && items.some(item => item.value === selected && !item.disabled) ? selected : items.find(item => !item.disabled)?.value ?? '';
        field.value = value;
        field.disabled = !items.some(item => !item.disabled);
    }
    const choice = () => choices[Number($('settings-account').value)];
    const model = () => models.find(m => m.id === $('settings-model').value);
    const key = (c, command) => `${prefix}${c.id}:${c.vessel_id}:${command.command_id}`;
    function reset() {
        choices = []; models = []; defaults = {};
        for (const id of ['settings-account','settings-model','settings-reasoning','settings-service']) options(id, []);
        $('settings-save').disabled = true;
    }
    function still(mine, c, client) { return mine === version && connection === c && c.client === client; }
    async function loadVessel() {
        if (saving) return;
        const mine = ++version; reset(); options('settings-workspace', []);
        connection = fleet.connections.get($('settings-vessel').value);
        const c = connection, client = c?.client;
        if (!client) return status('This Vessel is offline. Reconnect it to load workspaces and accounts.');
        status('Loading authorized workspaces…');
        try {
            const caps = await read(client, 'capabilities');
            if (!still(mine,c,client)) return;
            if (caps.vessel_id !== c.vessel_id) throw new Error('Vessel identity changed. Reconnect before continuing.');
            const required = mode === 'create' ? ['create','account_use'] : ['account_use'];
            if (required.some(right => !caps.rights?.includes(right))) throw new Error(`This connection needs ${required.join(' and ')} access. Ask the Vessel owner to update its connection grant.`);
            if (mode === 'create') {
                for (let i=0; i<localStorage.length; i++) if (localStorage.key(i)?.startsWith(`${prefix}${c.id}:${c.vessel_id}:`)) throw new Error('This Vessel has an unconfirmed creation. Close this form and use Check creation in the sidebar before creating another voyage.');
            }
            let workspaces = caps.workspaces || [];
            if (mode === 'edit') {
                const process = await read(client,'inspect',{session_id:target.session_id});
                if (!still(mine,c,client)) return;
                if (process.session_id !== target.session_id || process.incarnation !== target.incarnation) throw new Error('Voyage changed. Close this form and reopen its settings.');
                workspaces = [{name:process.workspace,path:process.workspace}];
            }
            options('settings-workspace',workspaces.map(w => ({value:w.path,label:w.name === w.path ? w.path : `${w.name} · ${w.path}`})));
            $('settings-workspace').disabled = mode === 'edit' || !workspaces.length;
            if (!workspaces.length) return status('No workspace is authorized for this connection. Add workspace access on the Vessel.');
            await loadAccounts();
        } catch (error) { if (still(mine,c,client)) status(error.message); }
    }
    async function loadAccounts() {
        if (saving) return;
        const mine = ++version; reset();
        const c = connection, client = c?.client, workspace = $('settings-workspace').value;
        if (!client || !workspace) return status('Choose a connected Vessel and workspace.');
        status('Loading provider accounts…');
        try {
            const catalogue = await read(client,'accounts',{workspace,transport:null});
            if (!still(mine,c,client)) return;
            if (catalogue.default_account) {
                defaults = await read(client,'account_defaults',{workspace});
                if (!still(mine,c,client)) return;
            }
            for (const account of catalogue.accounts || []) {
                const provider = catalogue.connections?.find(item => item.id === account.connection_id);
                for (const transport of provider?.transports || []) {
                    const binding = {account_id:account.id,connection_id:provider.id,identity_generation:account.identity_generation,connection_revision:provider.revision,transport};
                    const ready = account.state === 'ready' && account.availability === 'available';
                    choices.push({binding,ready,label:`${account.label} · ${provider.label} · ${transport.replaceAll('_',' ')}${same(binding,catalogue.default_account) ? ' · Default' : ''}${ready ? '' : ` · ${account.availability.replaceAll('_',' ')}`}`});
                }
            }
            const preferred = mode === 'edit' ? target.inference?.account : defaults.account;
            options('settings-account', choices.map((c,i) => ({value:String(i),label:c.label,disabled:!c.ready})), String(choices.findIndex(c => same(c.binding,preferred))));
            if (!choices.some(c => c.ready)) return status('No ready provider account is authorized here. Sign in or grant account access on this Vessel, then reload choices.');
            await loadModels();
        } catch (error) { if (still(mine,c,client)) status(error.message); }
    }
    async function loadModels() {
        if (saving) return;
        const mine = ++version, c = connection, client = c?.client, selected = choice();
        models = []; $('settings-save').disabled = true;
        for (const id of ['settings-model','settings-reasoning','settings-service']) options(id,[]);
        if (!client || !selected?.ready) return status('Choose an available provider account.');
        status('Loading models for this account…');
        try {
            const result = await read(client,'account_models',{workspace:$('settings-workspace').value,account:selected.binding});
            if (!still(mine,c,client)) return;
            if (!same(result.account,selected.binding) || !Array.isArray(result.models)) throw new Error('Account model list changed. Reload choices.');
            models = result.models;
            const seed = mode === 'edit' ? target.inference : defaults;
            const preferred = same(seed?.account,selected.binding) ? seed?.model : models.find(m => m.is_default)?.id;
            options('settings-model',models.map(m => ({value:m.id,label:m.display_name && m.display_name !== m.id ? `${m.display_name} · ${m.id}` : m.id})),preferred);
            updateModel();
        } catch (error) { if (still(mine,c,client)) status(error.message); }
    }
    function updateModel() {
        const selected = model();
        const seed = mode === 'edit' ? target.inference : defaults;
        const preserve = same(seed?.account,choice()?.binding) && seed?.model === selected?.id;
        for (const [id, values, original] of [['settings-reasoning',selected?.reasoning_efforts,seed?.reasoning_effort],['settings-service',selected?.service_tiers,seed?.service_tier]]) {
            const items = [{value:'',label:'Provider default'},...(values || []).map(v => ({value:v,label:v}))];
            // Existing explicit values remain visible; changing account/model deliberately resets them.
            if (preserve && original && !items.some(v => v.value === original)) items.push({value:original,label:`${original} · current`});
            options(id,items,preserve ? original || '' : '');
        }
        $('settings-save').disabled = !selected;
        status(selected ? mode === 'create' ? 'Create opens an empty voyage. Send your first message from its composer.' : 'Changes apply to the next run. Changing account or model resets its options to provider defaults.' : 'This account returned no models. Reload choices or choose another account.');
    }
    function open(edit) {
        if (saving) return;
        root.querySelector('[data-flux-sidebar-on-mobile]:not([data-flux-sidebar-collapsed-mobile]) [data-flux-sidebar-collapse] button')?.click();
        mode = edit ? 'edit' : 'create'; target = current();
        $('settings-retry').disabled = false;
        $('settings-close').disabled = false;
        $('settings-title').textContent = edit ? 'Account & model' : 'New voyage';
        $('settings-save').textContent = edit ? 'Apply settings' : 'Create voyage';
        $('settings-description').textContent = edit ? 'Choose the provider account and model for the next run.' : 'Choose where your voyage runs and which provider account it uses.';
        options('settings-vessel',[...fleet.connections.values()].map(c => ({value:c.id,label:`${c.name}${c.client ? '' : ' · offline'}`})),target.vessel);
        $('settings-vessel').disabled = edit;
        loadVessel();
    }
    async function accept(c, command, process, storageKey) {
        if (process?.session_id !== command.session_id || process.workspace !== command.workspace || typeof process.incarnation !== 'string') throw new Error('Creation identity could not be confirmed. Use Check creation.');
        localStorage.removeItem(storageKey);
        if (!c.voyages.some(v => v.session_id === process.session_id)) c.voyages.unshift(process);
        c.lastCatalogue = 0;
        select(c.id,process.session_id,process.name || 'New voyage');
        raw('settings-close').click();
        renderPending();
    }
    async function save(event) {
        event.preventDefault();
        if (saving || $('settings-save').disabled) return;
        const c = connection, client = c?.client, selected = choice(), selectedModel = model();
        if (!client || !selected?.ready || !selectedModel) return status('Reload choices before continuing.');
        const settings = {account:selected.binding,model:selectedModel.id,reasoning_effort:$('settings-reasoning').value || null,service_tier:$('settings-service').value || null};
        saving = true; ++version;
        status(mode === 'edit' ? 'Applying account and model settings…' : 'Creating voyage on this Vessel…');
        for (const field of $('voyage-settings-form').querySelectorAll('select,ui-select,button')) field.disabled = true;
        let recorded = false;
        try {
            if (mode === 'edit') {
                if (!await apply(target,settings)) throw new Error('Settings were not confirmed. Close this form to review the voyage status before trying again.');
                $('settings-close').disabled = false; $('settings-close').click();
            } else {
                const command = request('start_account',{command_id:uuid(),session_id:uuid(),workspace:$('settings-workspace').value,...settings}).command;
                const storageKey = key(c,command);
                // Store only immutable routing/account metadata, never credentials or message text.
                localStorage.setItem(storageKey,JSON.stringify({vessel:c.id,vessel_id:c.vessel_id,command})); recorded = true;
                const result = await client.exchange({protocol:1,command});
                if (result.protocol !== 1 || result.outcome_unknown !== false) throw new Error('Creation is unconfirmed. Close this form and use Check creation in the sidebar.');
                if (result.error != null) throw new Error('The Vessel did not confirm creation. Use Check creation to resolve this exact request before creating another voyage.');
                $('settings-close').disabled = false;
                await accept(c,command,result.result,storageKey);
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
                        if (result.status === 'created') await accept(c,record.command,result.process,storageKey);
                        else if (result.status === 'not_admitted') { localStorage.removeItem(storageKey); renderPending(); }
                        else throw new Error('Creation remains unconfirmed. Check again after reconnecting.');
                    } catch (error) { node.querySelector('[data-label]').textContent = clean(`${c.name} · ${error.message}`); node.disabled = !c.client; }
                });
                $('pending-creations').append(node);
            }
        } catch { status('Browser recovery storage is unavailable. Creation requires working local storage.'); }
    }
    raw('new-voyage').addEventListener('click',() => { $('settings-retry').disabled = false; open(false); });
    $('change-inference').addEventListener('click',() => { $('settings-retry').disabled = false; open(true); });
    for (const prefix of ['settings','edit']) {
        raw(`${prefix}-vessel`).addEventListener('change',loadVessel);
        raw(`${prefix}-workspace`).addEventListener('change',loadAccounts);
        raw(`${prefix}-account`).addEventListener('change',loadModels);
        raw(`${prefix}-model`).addEventListener('change',updateModel);
        raw(`${prefix}-retry`).addEventListener('click',loadVessel);
    }
    raw('voyage-settings-form').addEventListener('submit',save);
    raw('edit-save').addEventListener('click',save);
    raw('edit-close').addEventListener('click', () => raw('edit-popover').hidePopover?.());
    window.addEventListener('storage',renderPending);
    return {renderPending};
}
