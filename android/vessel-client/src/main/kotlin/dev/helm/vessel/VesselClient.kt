package dev.helm.vessel

import java.io.Closeable
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.serialization.json.*

/** RESOLVED means admission/refusal observed, NOT remote execution/cleanup complete. */
enum class JournalState { UNCERTAIN, RESOLVED }
class JournalEntry(
    val vesselId: String,
    val sessionId: String,
    val commandId: String,
    val exactRequest: String,
    val state: JournalState = JournalState.UNCERTAIN,
    val responseJson: String? = null,
) {
    override fun toString() = "JournalEntry(commandId=$commandId, state=$state)"
}

/**
 * Android adapter must commit durably before returning from insert. Atomic uniqueness is commandId.
 * If present, compare vesselId/sessionId/exactRequest byte-for-byte: throw on any mismatch, false
 * only for an identical intent. Never replace a saved payload. record updates only observation fields
 * atomically and must not regress RESOLVED to UNCERTAIN. Records contain private text: private storage,
 * no diagnostics or backups; credentials and socket IDs never belong in this journal.
 */
interface MutationJournal {
    suspend fun insert(entry: JournalEntry): Boolean
    suspend fun record(commandId: String, state: JournalState, responseJson: String?)
    suspend fun pending(vesselId: String): List<JournalEntry>
}

class VesselClient(
    private val endpoint: VesselEndpoint,
    private val journal: MutationJournal,
    private val transport: VesselTransport = OkHttpVesselTransport(endpoint),
) : Closeable {
    val state: StateFlow<ConnectionState> get() = transport.state
    suspend fun connect() = transport.connect()
    /** Whitelist prevents accidental mutation bypass of durable intent storage. */
    suspend fun read(request: JsonObject): VesselResponse {
        require(request["protocol"]?.jsonPrimitive?.int == 1)
        require(request.getValue("command").jsonObject.string("op") in READ_OPERATIONS) { "Use mutate for durable operations" }
        return transport.exchange(request.toString())
    }
    suspend fun mutate(mutation: Mutation): VesselResponse {
        val exact = mutation.request.toString()
        require(exact.toByteArray(Charsets.UTF_8).size <= MAX_FRAME_BYTES - 128) { "Request limit" }
        val entry = JournalEntry(endpoint.vesselId, mutation.sessionId, mutation.commandId, exact)
        // Process death at ANY point after this commit leaves a recoverable uncertainty, even before send.
        // Repeated button press with same ID never repeats external dispatch.
        if (!journal.insert(entry)) return recover(entry)
        val response = transport.exchange(exact)
        record(entry, response, receipt = false)
        return response
    }
    suspend fun recover(entry: JournalEntry): VesselResponse {
        require(entry.vesselId == endpoint.vesselId) { "Journal belongs to another Vessel" }
        uuid(entry.sessionId); uuid(entry.commandId)
        val response = read(Requests.receipt(entry.sessionId, entry.commandId))
        record(entry, response, receipt = true)
        return response
    }
    /** Bounded recovery page; caller chooses subsequent pages. Never resend or invoke resolve(original). */
    suspend fun recoverPending(limit: Int = 32): List<VesselResponse> {
        require(limit in 1..128)
        return journal.pending(endpoint.vesselId).take(limit).map { recover(it) }
    }
    private suspend fun record(entry: JournalEntry, response: VesselResponse, receipt: Boolean) {
        val envelope = response.result as? JsonObject
        val result = envelope?.get("result") as? JsonObject
        val identityMatches = envelope?.get("session_id")?.jsonPrimitive?.contentOrNull == entry.sessionId &&
            (result?.get("command_id")?.jsonPrimitive?.contentOrNull == entry.commandId ||
                (result?.get("request") as? JsonObject)?.get("receipt_id")?.jsonPrimitive?.contentOrNull == entry.commandId ||
                ((result?.get("record") as? JsonObject)?.get("request") as? JsonObject)?.get("receipt_id")?.jsonPrimitive?.contentOrNull == entry.commandId)
        // A successful receipt with unknown status is NOT evidence of non-delivery.
        // Known receipts can be accepted/requested/queued: admission resolved, execution still pending.
        val resolved = !response.outcomeUnknown && if (!receipt) {
            response.error != null || identityMatches
        } else {
            response.error == null && identityMatches && result?.get("status")?.jsonPrimitive?.contentOrNull in RECEIPT_STATUSES
        }
        journal.record(entry.commandId, if (resolved) JournalState.RESOLVED else JournalState.UNCERTAIN,
            obj("protocol" to 1.json(), "result" to response.result, "error" to (response.error?.json() ?: JsonNull), "outcome_unknown" to JsonPrimitive(response.outcomeUnknown)).toString())
    }
    fun observe(subscriptions: List<EventSubscription>): Flow<VesselEvent> = transport.observe(subscriptions)

    /** Canonical recent snapshot + revision-pinned page. Never append invalidation events as text.
     * A race/revision change fails; caller keeps cache stale and repeats reconciliation deliberately.
     * Large history is paged separately. Full message chunks use UTF-8 byte offsets from server.
     */
    suspend fun reconcile(sessionId: String, historyOffset: Long = 0, historyLimit: Int = 64): Reconciliation {
        val generation = state.value
        require(generation.socketId != null) { "Vessel not connected" }
        val snapshotReply = read(Requests.snapshot(sessionId)).voyageResult(sessionId)
        val snapshot = snapshotReply.getValue("result").jsonObject
        val revision = snapshot.number("revision")
        val historyReply = read(Requests.history(sessionId, historyOffset, historyLimit, revision)).voyageResult(sessionId)
        val history = historyReply.getValue("result").jsonObject
        require(snapshot.string("session_id") == sessionId && history.string("session_id") == sessionId && history.number("revision") == revision && snapshotReply.string("incarnation") == historyReply.string("incarnation")) { "Canonical identity changed; reconcile again" }
        require(state.value == generation) { "Connection changed; cache remains stale" }
        return Reconciliation(sessionId, snapshotReply.string("incarnation"), revision, snapshot.number("observation_cursor"), snapshot, history, generation)
    }
    override fun close() = transport.close()
    companion object {
        private val READ_OPERATIONS = setOf("capabilities", "catalogue", "inspect", "snapshot", "history", "message_chunk", "run_output", "events", "decisions", "receipt")
        private val RECEIPT_STATUSES = setOf("accepted", "requested", "already_terminal", "applied", "deleted", "transferred", "queued", "not_applied", "unknown_after_restart")
    }
}

/** Opaque JSON projections retain new fields instead of inventing narrowed snapshot wire shapes. */
class Reconciliation(val sessionId: String, val incarnation: String, val revision: Long, val cursor: Long,
                     val snapshot: JsonObject, val history: JsonObject, val connection: ConnectionState) {
    override fun toString() = "Reconciliation(sessionId=$sessionId, revision=$revision)"
}

fun VesselResponse.voyageResult(sessionId: String): JsonObject {
    if (!successful) throw VesselFailure("Vessel refused or could not confirm operation")
    val reply = result.jsonObject
    require(reply.string("session_id") == sessionId) { "Vessel response identity mismatch" }
    uuid(reply.string("incarnation"))
    return reply
}
