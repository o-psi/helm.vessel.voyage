package dev.helm.vessel

import java.util.UUID
import kotlinx.serialization.json.*

internal val wireJson = Json { ignoreUnknownKeys = true }
internal fun obj(vararg fields: Pair<String, JsonElement>): JsonObject = JsonObject(mapOf(*fields))
internal fun String.json() = JsonPrimitive(this)
internal fun Long.json() = JsonPrimitive(this)
internal fun Int.json() = JsonPrimitive(this)
internal fun uuid(value: String): String {
    require(runCatching { UUID.fromString(value).toString() == value.lowercase() && UUID.fromString(value) != UUID(0, 0) }.getOrDefault(false)) { "Invalid identity" }
    return value
}
internal fun JsonObject.string(key: String) = getValue(key).jsonPrimitive.content
internal fun JsonObject.number(key: String) = getValue(key).jsonPrimitive.long

/** Deliberately opaque diagnostics: results/prompts can contain private conversation text. */
class VesselResponse(val result: JsonElement, val error: String?, val outcomeUnknown: Boolean) {
    val successful: Boolean get() = error == null && !outcomeUnknown
    override fun toString() = "VesselResponse(successful=$successful, outcomeUnknown=$outcomeUnknown)"
    internal companion object {
        fun decode(value: JsonObject): VesselResponse {
            require(value.number("protocol") == 1L) { "Unsupported Vessel protocol" }
            return VesselResponse(value["result"] ?: JsonNull,
                value["error"]?.jsonPrimitive?.contentOrNull,
                value["outcome_unknown"]?.jsonPrimitive?.boolean ?: true)
        }
    }
}

/** Wire envelopes mirror crates/voyage-protocol/src/{vessel,duplex}.rs (protocol 1). */
object Requests {
    fun capabilities() = request(obj("op" to "capabilities".json()))
    fun catalogue() = request(obj("op" to "catalogue".json()))
    fun inspect(sessionId: String) = session("inspect", sessionId)
    fun snapshot(sessionId: String) = session("snapshot", sessionId)
    fun decisions(sessionId: String) = session("decisions", sessionId)
    fun receipt(sessionId: String, commandId: String) = session("receipt", sessionId, "command_id" to uuid(commandId).json())
    fun history(sessionId: String, offset: Long = 0, limit: Int = 64, expectedRevision: Long? = null): JsonObject {
        require(offset >= 0 && limit in 1..128 && (expectedRevision == null || expectedRevision >= 0))
        return session("history", sessionId, "offset" to offset.json(), "limit" to limit.json(), "expected_revision" to (expectedRevision?.json() ?: JsonNull))
    }
    fun events(sessionId: String, incarnation: String, after: Long, limit: Int = 64, waitMs: Int = 0): JsonObject {
        require(after >= 0 && limit in 1..128 && waitMs in 0..10000)
        return session("events", sessionId, "incarnation" to uuid(incarnation).json(), "after" to after.json(), "limit" to limit.json(), "wait_ms" to waitMs.json())
    }
    fun messageChunk(sessionId: String, index: Long, offset: Long, expectedRevision: Long, limit: Int = 65536): JsonObject {
        require(index >= 0 && offset >= 0 && expectedRevision >= 0 && limit in 1..65536)
        return session("message_chunk", sessionId, "index" to index.json(), "offset" to offset.json(), "expected_revision" to expectedRevision.json(), "limit" to limit.json())
    }
    fun runOutput(sessionId: String, runId: String, offset: Long = 0, limit: Int = 65536): JsonObject {
        require(offset >= 0 && limit in 1..65536)
        return session("run_output", sessionId, "run_id" to uuid(runId).json(), "offset" to offset.json(), "limit" to limit.json())
    }
    private fun request(command: JsonObject) = obj("protocol" to 1.json(), "command" to command)
    internal fun session(op: String, id: String, vararg fields: Pair<String, JsonElement>) = request(obj("op" to op.json(), "session_id" to uuid(id).json(), *fields))
}

/** One user intent. Expiry is an absolute Unix millisecond deadline, not renewed on recovery. */
class Mutation private constructor(val sessionId: String, val commandId: String, val request: JsonObject) {
    override fun toString() = "Mutation(commandId=$commandId)"
    companion object {
        fun submit(sessionId: String, commandId: String, revision: Long, expiresAtMs: Long, prompt: String) =
            make("submit", sessionId, commandId, revision, expiresAtMs, "prompt" to promptValue(prompt))
        fun steer(sessionId: String, incarnation: String, commandId: String, revision: Long, expiresAtMs: Long, runId: String, prompt: String) =
            make("steer", sessionId, commandId, revision, expiresAtMs, "incarnation" to uuid(incarnation).json(), "run_id" to uuid(runId).json(), "prompt" to promptValue(prompt))
        fun cancel(sessionId: String, incarnation: String, commandId: String, revision: Long, expiresAtMs: Long, runId: String) =
            make("cancel", sessionId, commandId, revision, expiresAtMs, "incarnation" to uuid(incarnation).json(), "run_id" to uuid(runId).json())
        fun respond(sessionId: String, incarnation: String, commandId: String, revision: Long, expiresAtMs: Long, runId: String, decisionId: String, response: JsonElement) =
            make("respond", sessionId, commandId, revision, expiresAtMs, "incarnation" to uuid(incarnation).json(), "run_id" to uuid(runId).json(), "decision_id" to uuid(decisionId).json(), "response" to response)
        private fun promptValue(prompt: String): JsonPrimitive {
            require(prompt.isNotBlank() && prompt.toByteArray(Charsets.UTF_8).size <= 65536) { "Invalid prompt size" }
            return prompt.json()
        }
        private fun make(op: String, sessionId: String, commandId: String, revision: Long, expiry: Long, vararg fields: Pair<String, JsonElement>): Mutation {
            require(revision >= 0 && expiry > 0)
            return Mutation(uuid(sessionId), uuid(commandId), Requests.session(op, sessionId, "command_id" to commandId.json(), "expected_revision" to revision.json(), "expires_at_ms" to expiry.json(), *fields))
        }
    }
}

class VesselFailure(message: String) : Exception(message)
