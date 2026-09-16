import {connectionDiagnostic as log} from './connection-diagnostics.js';
import {request, IntentJournal, VesselSocket} from './vessel-client.js';

// Every configured connection owns its own socket, catalogue, lease and journal.
export class VesselFleet {
    constructor(vessels, {tenantId, ticket, changed}) {
        Object.assign(this, {tenantId, ticket, changed});
        this.connections = new Map(vessels.map(vessel => [vessel.id, {
            ...vessel, client: null, journal: null, voyages: [], status: 'Connecting…',
            generation: 0, retry: 1000, lastCatalogue: 0, polling: false, stopped: false,
        }]));
        this.closed = false;
    }
    start() { for (const connection of this.connections.values()) this.connect(connection); }
    async connect(connection, renewing = false) {
        if (this.closed || connection.connecting) return;
        connection.connecting = true;
        const generation = connection.generation;
        const previous = connection.client;
        const current = () => !this.closed && generation === connection.generation;
        clearTimeout(connection.reconnect); clearTimeout(connection.renewal);
        if (!previous) { connection.status = 'Connecting…'; this.changed(); }
        let socket;
        try {
            const auth = await this.ticket(connection.id);
            if (!current()) return;
            const url = new URL(auth.url);
            if (url.protocol !== 'wss:' || url.username || url.password || url.search || url.hash
                || url.pathname !== '/v1/vessel/browser-socket' || (url.port && url.port !== '443')
                || auth.vessel_id !== connection.vessel_id || typeof auth.token !== 'string'
                || !/^[a-f0-9]{64}$/i.test(auth.token) || !Number.isSafeInteger(auth.expires_at_ms)
                || auth.expires_at_ms <= Date.now() + 10000 || auth.expires_at_ms > Date.now() + 125000) {
                throw Object.assign(new Error('Invalid Vessel authorization'), {permanent:true});
            }
            socket = new WebSocket(url, 'voyage.vessel.v1');
            connection.opening = socket;
            await new Promise((resolve, reject) => {
                const finish = error => {
                    clearTimeout(timer); socket.removeEventListener('message', receive);
                    socket.removeEventListener('close', closed);
                    error ? reject(error) : resolve();
                };
                const closed = () => finish(new Error('Vessel unavailable'));
                const receive = event => {
                    try {
                        const frame = JSON.parse(event.data);
                        if (socket.protocol !== 'voyage.vessel.v1' || frame.type !== 'hello' || frame.protocol !== 1
                            || frame.vessel_id !== connection.vessel_id || typeof frame.socket_id !== 'string') throw Error();
                        finish();
                    } catch { finish(new Error('Vessel authentication failed')); socket.close(); }
                };
                const timer = setTimeout(() => { finish(new Error('Connection timed out')); socket.close(); }, 10000);
                socket.addEventListener('open', () => socket.send(JSON.stringify({type:'authenticate', token:auth.token})), {once:true});
                socket.addEventListener('message', receive);
                socket.addEventListener('close', closed, {once:true});
                socket.addEventListener('error', () => socket.close());
            });
            if (!current()) { socket.close(); return; }
            try {
                connection.journal ||= new IntentJournal(localStorage, `${this.tenantId}:${connection.id}:${auth.vessel_id}`);
                connection.journal.entries();
            } catch { throw Object.assign(new Error('Command journal unavailable'), {permanent:true}); }
            const client = new VesselSocket(socket, reason => {
                if (!current() || connection.client !== client) return;
                connection.client = null; connection.status = `Offline · ${reason}`;
                clearTimeout(connection.renewal); this.changed(); this.schedule(connection);
            });
            connection.socket = socket; connection.client = client; connection.opening = null;
            // New reads/commands use the replacement. Already dispatched commands
            // keep receiving replies on the old socket; nothing is replayed.
            if (previous) {
                connection.draining ||= new Set(); connection.draining.add(previous);
                previous.drain().finally(() => connection.draining.delete(previous));
            }
            connection.retry = 1000; connection.lastCatalogue = 0; connection.polling = false;
            connection.status = 'Connected';
            connection.renewal = setTimeout(() => this.connect(connection, true), Math.max(1000, auth.expires_at_ms - Date.now() - 30000));
            log(renewing ? 'renewal_ack' : 'connected',{connection:connection.id,generation});
            this.changed(); await this.catalogue(connection);
        } catch (error) {
            socket?.close();
            if (!current()) return;
            connection.stopped ||= Boolean(error.permanent);
            if (connection.stopped) { connection.client?.close(); connection.client = null; }
            connection.status = error.message; this.changed(); this.schedule(connection);
        } finally {
            connection.connecting = false;
            if (connection.opening === socket) connection.opening = null;
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
            connection.generation++; clearTimeout(connection.reconnect); clearTimeout(connection.renewal); connection.opening?.close(); connection.socket?.close();
            for (const client of connection.draining || []) client.close();
        }
    }
}
