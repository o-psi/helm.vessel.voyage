package dev.helm.android

import android.app.Application
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.SavedStateHandle
import androidx.lifecycle.viewModelScope
import dev.helm.vessel.*
import java.util.UUID
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*

internal fun JsonElement?.obj() = this as? JsonObject ?: JsonObject(emptyMap())
internal fun JsonElement?.arr() = this as? JsonArray ?: JsonArray(emptyList())
internal fun JsonObject.text(key: String) = (get(key) as? JsonPrimitive)?.contentOrNull.orEmpty()
internal fun JsonObject.number(key: String) = (get(key) as? JsonPrimitive)?.longOrNull ?: 0L
internal fun JsonObject.flag(key: String) = (get(key) as? JsonPrimitive)?.booleanOrNull == true
internal fun displayText(text: String) = text.filter { it == '\n' || it == '\t' || (!it.isISOControl() && it !in '\u202a'..'\u202e' && it !in '\u2066'..'\u2069') }

data class HelmState(
    val configured: Boolean = false, val connected: Boolean = false, val busy: Boolean = false,
    val stale: Boolean = true, val notice: String = "Configure an existing trusted WSS Vessel.",
    val voyages: List<JsonObject> = emptyList(), val selected: String? = null,
    val snapshot: JsonObject = JsonObject(emptyMap()), val incarnation: String = "",
    val messages: List<JsonObject> = emptyList(), val decisions: List<JsonObject> = emptyList(),
    val earlierOffset: Long = 0, val pending: List<String> = emptyList(), val savedAt: Long = 0,
    val decisionAvailable: Boolean = false, val actionStatus: String = "",
) {
    val run get() = snapshot["run"].obj()
    val running get() = run.text("state") in setOf("running", "cancelling", "starting")
    val actionable get() = connected && !stale && !busy && pending.isEmpty()
}

class HelmViewModel(application: Application, private val saved: SavedStateHandle) : AndroidViewModel(application) {
    private val db = HelmDatabase.open(application)
    private val journal = RoomJournal(db)
    private val credentials = CredentialStore(application)
    internal var clientFactory: (VesselEndpoint, MutationJournal) -> VesselClient = { endpoint, journal -> VesselClient(endpoint, journal) }
    private val mutable = MutableStateFlow(HelmState(selected = saved["selected"]))
    val state = mutable.asStateFlow()
    val draft = saved.getStateFlow("draft", "")
    private var client: VesselClient? = null
    private var vesselId = ""
    private var foreground = false
    private var mutationInFlight = false
    private var activation: Job? = null
    private var observation: Job? = null
    private var socketWatch: Job? = null
    private val lock = Mutex()
    fun draft(value: String) { saved["draft"] = value }
    fun resume() { foreground = true; if (client == null) activate() }
    fun pause() {
        foreground = false
        activation?.cancel(); observation?.cancel(); socketWatch?.cancel()
        client?.close(); client = null
        mutable.update { it.copy(connected = false, stale = true, busy = false, notice = "Disconnected locally; remote execution is not cancelled.") }
    }
    fun reconnect() { pause(); resume() }
    fun configure(url: String, vessel: String, grant: String, token: String) {
        viewModelScope.launch {
            try {
                VesselEndpoint(url.trim(), vessel.trim(), grant.trim(), token.trim())
                withContext(Dispatchers.IO) { credentials.save(url.trim(), vessel.trim(), grant.trim(), token.trim()) }
                pause(); mutable.value = HelmState(configured = true); saved["selected"] = null; resume()
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                mutable.update { it.copy(notice = "Connection not saved. Check WSS URL, UUID identities, scoped token and device storage.") }
            }
        }
    }
    fun forget() {
        pause()
        viewModelScope.launch {
            try { withContext(Dispatchers.IO) { credentials.forget() }; saved["selected"] = null; saved["draft"] = ""; mutable.value = HelmState() }
            catch (e: Exception) { mutable.update { it.copy(notice = "Could not remove saved credentials.") } }
        }
    }
    private fun activate() {
        activation?.cancel()
        activation = viewModelScope.launch {
            try {
                val connection = withContext(Dispatchers.IO) { credentials.load() } ?: return@launch
                if (!foreground) return@launch
                vesselId = connection.text("vessel")
                val active = clientFactory(VesselEndpoint(connection.text("url"), vesselId, connection.text("grant"), connection.text("token")), journal)
                client = active
                mutable.update { it.copy(configured = true, busy = true, stale = true, notice = "Connecting and reconciling…") }
                loadCache()
                active.connect()
                socketWatch = launch {
                    active.state.collect { socket ->
                        if (client === active && socket.socketId == null) mutable.update { it.copy(connected = false, stale = true, notice = "Connection lost. Reconnect to reconcile; uncertain commands are not resent.") }
                    }
                }
                mutable.update { it.copy(connected = true) }
                recover(active)
                refresh(active)
                // Observation is invalidation only. Periodic reconciliation also handles decisions and owner changes.
                while (isActive && client === active) {
                    delay(3000)
                    if (active.state.value.socketId == null) break
                    refresh(active)
                }
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                mutable.update { it.copy(stale = true, connected = client?.state?.value?.socketId != null, notice = "Connection or reconciliation failed. Cached content is stale; check grant, trust and network, then reconnect.") }
            } finally { mutable.update { it.copy(busy = mutationInFlight) } }
        }
    }
    private suspend fun loadCache() {
        val cacheVessel = vesselId
        db.dao().cached(cacheVessel, "catalogue")?.let { row -> if (vesselId == cacheVessel) mutable.update { it.copy(voyages = Json.parseToJsonElement(row.json).arr().map { it.obj() }, savedAt = row.savedAt) } }
        val id = state.value.selected ?: return
        db.dao().cached(cacheVessel, "snapshot:$id")?.let { row ->
            val snapshot = Json.parseToJsonElement(row.json).obj()
            if (state.value.selected == id && vesselId == cacheVessel) mutable.update { it.copy(snapshot = snapshot, messages = snapshot["messages"].arr().map { it.obj() }, earlierOffset = snapshot.number("message_offset"), savedAt = row.savedAt, stale = true) }
        }
    }
    private suspend fun recover(active: VesselClient) {
        // Bounded receipt reads only. An unknown receipt is retained, never a new dispatch.
        journal.pending(vesselId).take(128).forEach { active.recover(it) }
        updatePending()
    }
    private suspend fun updatePending() {
        val pending = journal.pending(vesselId)
        mutable.update { current -> current.copy(pending = pending.filter { it.sessionId == current.selected }.map { it.commandId }) }
    }
    fun recoverReceipts() { viewModelScope.launch { guarded { client?.let { recover(it); refresh(it) } } } }
    private suspend fun guarded(block: suspend () -> Unit) {
        try { block() } catch (e: Exception) {
            if (e is CancellationException) throw e
            mutable.update { it.copy(stale = true, notice = "Operation could not be confirmed. Reconcile and check receipts; do not repeat uncertain actions.") }
        }
    }
    fun select(id: String?) {
        observation?.cancel(); observation = null
        saved["selected"] = id
        mutable.update { it.copy(selected = id, snapshot = JsonObject(emptyMap()), messages = emptyList(), decisions = emptyList(), earlierOffset = 0, stale = true, pending = emptyList()) }
        viewModelScope.launch { guarded { loadCache(); updatePending(); client?.let { refresh(it) } } }
    }
    fun refresh() { viewModelScope.launch { guarded { client?.let { refresh(it) } } } }
    private suspend fun refresh(active: VesselClient): Unit = lock.withLock {
        if (client !== active || !foreground) return@withLock
        val cacheVessel = vesselId
        val catalogue = active.read(Requests.catalogue())
        check(catalogue.successful)
        if (client !== active || !foreground) return@withLock
        val voyages = catalogue.result.arr().map { it.obj() }
        db.dao().cache(CacheRow(cacheVessel, "catalogue", catalogue.result.toString(), System.currentTimeMillis()))
        if (client !== active || !foreground) return@withLock
        mutable.update { it.copy(voyages = voyages) }
        val id = state.value.selected
        if (id == null) {
            mutable.update { it.copy(stale = false, busy = false, notice = "Connected. Select a voyage.") }
            return@withLock
        }
        val reconciliation = active.reconcile(id)
        val decisionsResponse = active.read(Requests.decisions(id))
        val decisions = if (decisionsResponse.successful) {
            val reply = decisionsResponse.voyageResult(id)
            check(reply.text("incarnation") == reconciliation.incarnation)
            reply["result"].arr().map { it.obj() }
        } else emptyList()
        if (client !== active || state.value.selected != id || active.state.value != reconciliation.connection) return@withLock
        val snapshot = reconciliation.snapshot
        val previous = state.value
        val changed = previous.snapshot.number("revision") != reconciliation.revision
        val now = System.currentTimeMillis()
        db.dao().cache(CacheRow(cacheVessel, "snapshot:$id", snapshot.toString(), now))
        if (client !== active || state.value.selected != id || active.state.value != reconciliation.connection) return@withLock
        mutable.update { it.copy(snapshot = snapshot, incarnation = reconciliation.incarnation,
            messages = if (changed || it.messages.isEmpty()) snapshot["messages"].arr().map { m -> m.obj() } else it.messages,
            earlierOffset = if (changed || it.messages.isEmpty()) snapshot.number("message_offset") else it.earlierOffset,
            decisions = decisions, decisionAvailable = decisionsResponse.successful,
            stale = false, busy = mutationInFlight, connected = true, savedAt = now, notice = "Canonical revision ${reconciliation.revision}. Live text is provisional.") }
        updatePending()
        if (observation == null || previous.incarnation != reconciliation.incarnation) {
            observation?.cancel()
            observation = viewModelScope.launch {
                guarded {
                    active.observe(listOf(EventSubscription(id, reconciliation.incarnation, reconciliation.cursor))).buffer(kotlinx.coroutines.channels.Channel.CONFLATED).collect {
                        if (client === active && state.value.selected == id) refresh(active)
                    }
                }
            }
        }
    }
    fun earlier() { viewModelScope.launch { guarded { lock.withLock {
        val before = state.value; val active = client ?: return@withLock; val id = before.selected ?: return@withLock
        val offset = (before.earlierOffset - 64).coerceAtLeast(0)
        val messages = mutableListOf<JsonObject>()
        var next = offset
        while (next < before.earlierOffset) {
            val page = active.read(Requests.history(id, next, 64, before.snapshot.number("revision"))).voyageResult(id)["result"].obj()
            messages += page["messages"].arr().map { it.obj() }
            check(page.number("next_offset") > next)
            next = page.number("next_offset")
        }
        if (state.value.selected == id && state.value.snapshot.number("revision") == before.snapshot.number("revision")) mutable.update {
            it.copy(messages = (messages + it.messages).distinctBy { m -> m.number("message_index") }, earlierOffset = offset)
        }
    } } } }
    fun expand(index: Long) { viewModelScope.launch { guarded { lock.withLock {
        val before = state.value; val active = client ?: return@withLock; val id = before.selected ?: return@withLock
        val content = StringBuilder()
        var offset = 0L
        do {
            val page = active.read(Requests.messageChunk(id, index, offset, before.snapshot.number("revision"))).voyageResult(id)["result"].obj()
            if (page.number("total_bytes") > 4 * 1024 * 1024) {
                mutable.update { it.copy(notice = "This message exceeds the Android 4 MiB expanded-message limit. Use another Helm client for the full message.") }
                return@withLock
            }
            content.append(page.text("data"))
            val next = page.number("next_offset")
            check(next > offset || !page.flag("has_more"))
            offset = next
        } while (page.flag("has_more"))
        val full = Json.parseToJsonElement(content.toString()).obj().toMutableMap().apply {
            put("message_index", JsonPrimitive(index)); put("projection_truncated", JsonPrimitive(false))
        }
        if (state.value.selected == id && state.value.snapshot.number("revision") == before.snapshot.number("revision")) mutable.update {
            it.copy(messages = it.messages.map { m -> if (m.number("message_index") == index) JsonObject(full) else m })
        }
    } } } }
    fun send(kind: String, decision: JsonObject? = null, response: JsonElement? = null) {
        val before = state.value
        if (!before.actionable || mutationInFlight) return
        val id = before.selected ?: return
        val active = client ?: return
        val prompt = draft.value
        mutationInFlight = true
        mutable.update { it.copy(busy = true) }
        viewModelScope.launch {
            try {
                val command = UUID.randomUUID().toString()
                val revision = before.snapshot.number("revision")
                val expires = System.currentTimeMillis() + 60_000
                val mutation = when (kind) {
                    "submit" -> Mutation.submit(id, command, revision, expires, prompt)
                    "steer" -> Mutation.steer(id, before.incarnation, command, revision, expires, before.run.text("run_id"), prompt)
                    "cancel" -> Mutation.cancel(id, before.incarnation, command, revision, expires, before.run.text("run_id"))
                    "respond" -> {
                        requireNotNull(decision); requireNotNull(response)
                        Mutation.respond(id, decision.text("incarnation"), command, revision, minOf(expires, decision.number("expires_at_ms")), decision.text("run_id"), decision.text("decision_id"), response)
                    }
                    else -> error("Unsupported action")
                }
                val result = active.mutate(mutation)
                if (client === active && state.value.selected == id && draft.value == prompt && result.successful && kind in setOf("submit", "steer")) saved["draft"] = ""
                if (client === active) mutable.update { it.copy(actionStatus = "$command · " + if (result.successful) "Acknowledged; remote execution may still be pending." else if (result.outcomeUnknown) "Outcome uncertain. Check receipts before another action." else "Refused by Vessel; no success claimed.") }
                refresh(active)
            } catch (e: Exception) {
                if (e is CancellationException) throw e
                mutable.update { it.copy(stale = true, actionStatus = "Action not confirmed. Any saved intent is retained; recovery reads receipts only.") }
            } finally { mutationInFlight = false; updatePending(); mutable.update { it.copy(busy = false) } }
        }
    }
    override fun onCleared() { client?.close(); db.close() }
}
