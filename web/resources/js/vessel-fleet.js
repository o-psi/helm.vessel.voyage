import {connectionDiagnostic as log} from './connection-diagnostics.js';
import {request, IntentJournal, VesselSocket} from './vessel-client.js';

// Every configured connection owns its own socket, catalogue, lease and journal.
export class VesselFleet {
    constructor(vessels, {tenantId, socketPath, ticket, changed}) {
        Object.assign(this, {tenantId, socketPath, ticket, changed});
        this.connections = new Map(vessels.map(vessel => [vessel.id, {
            ...vessel, client: null, journal: null, voyages: [], status: 'Connecting…',
            generation: 0, retry: 1000, lastCatalogue: 0, polling: false, stopped: false,
        }]));
        this.closed = false;
    }
    start() { for (const connection of this.connections.values()) this.connect(connection); }
    async connect(connection) {
        if (this.closed) return;
        const generation = ++connection.generation;
        log('connect_start',{connection:connection.id,generation});
        clearTimeout(connection.reconnect); clearInterval(connection.renewal);
        connection.socket?.close(); connection.client = null; connection.polling = false;
        connection.status = 'Connecting…'; this.changed();
        const current = () => !this.closed && generation === connection.generation;
        try {
            const auth = await this.ticket(connection.id);
            if (!current()) return;
            const url = new URL(this.socketPath, location.href);
            url.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
            const socket = new WebSocket(url); connection.socket = socket;
            const ready = await new Promise((resolve, reject) => {
                const timer = setTimeout(() => { socket.close(); reject(new Error('Connection timed out')); }, 10000);
                socket.addEventListener('open', () => socket.send(JSON.stringify({type:'authenticate', ticket:auth})));
                socket.addEventListener('message', function receive(event) {
                    try {
                        const frame = JSON.parse(event.data);
                        if (frame.type === 'ready') { clearTimeout(timer); socket.removeEventListener('message', receive); resolve(frame); }
                    } catch { socket.close(); }
                });
                socket.addEventListener('close', () => { clearTimeout(timer); reject(new Error('Unavailable')); }, {once:true});
                socket.addEventListener('error', () => socket.close());
            });
            if (!current()) { socket.close(); return; }
            if (ready.vessel_id !== connection.vessel_id) {
                connection.stopped = true; socket.close(); throw new Error('Vessel identity changed');
            }
            try {
                connection.journal = new IntentJournal(localStorage, `${this.tenantId}:${connection.id}:${ready.vessel_id}`);
                connection.journal.entries();
            } catch {
                connection.stopped = true; socket.close(); throw new Error('Command journal unavailable');
            }
            connection.client = new VesselSocket(socket, reason => {
                if (!current()) return;
                connection.client = null; connection.status = `Offline · ${reason}`;
                clearInterval(connection.renewal); this.changed(); this.schedule(connection);
            });
            log('connected',{connection:connection.id,generation});
            let renewalStarted = 0;
            socket.addEventListener('message',event => {
                try { if (JSON.parse(event.data).type === 'ready' && renewalStarted) { log('renewal_ack',{connection:connection.id,elapsed_ms:Date.now()-renewalStarted}); renewalStarted = 0; } } catch {}
            });
            connection.retry = 1000; connection.lastCatalogue = 0; connection.status = 'Connected';
            connection.renewal = setInterval(async () => {
                try {
                    renewalStarted = Date.now(); log('renewal_start',{connection:connection.id});
                    const ticket = await this.ticket(connection.id);
                    if (current() && socket.readyState === 1) socket.send(JSON.stringify({type:'authenticate', ticket}));
                } catch (error) {
                    if (!current()) return;
                    log('renewal_failed',{connection:connection.id,elapsed_ms:Date.now()-renewalStarted});
                    connection.stopped = Boolean(error.permanent); socket.close();
                }
            }, 30000);
            this.changed(); await this.catalogue(connection);
        } catch (error) {
            if (!current()) return;
            connection.client = null; connection.status = error.message;
            connection.stopped ||= Boolean(error.permanent);
            this.changed(); this.schedule(connection);
        }
    }
    schedule(connection) {
        if (this.closed || connection.stopped) return;
        clearTimeout(connection.reconnect);
        log('reconnect_scheduled',{connection:connection.id,delay_ms:connection.retry});
        connection.reconnect = setTimeout(() => this.connect(connection), connection.retry);
        connection.retry = Math.min(connection.retry * 2, 15000);
    }
    async catalogue(connection) {
        if (!connection.client || connection.polling || Date.now() - connection.lastCatalogue < 10000) return;
        const client = connection.client, generation = connection.generation;
        connection.polling = true;
        try {
            const response = await client.exchange(request('catalogue'));
            if (response.protocol !== 1 || response.error != null || response.outcome_unknown !== false || !Array.isArray(response.result)) throw new Error('Voyage list unavailable');
            if (generation !== connection.generation || client !== connection.client || this.closed) return;
            connection.voyages = response.result;
            connection.status = 'Connected';
        } catch {
            if (generation !== connection.generation || client !== connection.client || this.closed) return;
            connection.status = 'Voyage list unavailable';
        } finally {
            if (generation === connection.generation) {
                connection.polling = false; connection.lastCatalogue = Date.now(); this.changed();
            }
        }
    }
    poll() { for (const connection of this.connections.values()) this.catalogue(connection); }
    reconnect() {
        for (const connection of this.connections.values()) { connection.stopped = false; this.connect(connection); }
    }
    close() {
        this.closed = true;
        for (const connection of this.connections.values()) {
            connection.generation++; clearTimeout(connection.reconnect); clearInterval(connection.renewal); connection.socket?.close();
        }
    }
}
