package dev.helm.vessel

import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*

internal const val SESSION = "11111111-1111-4111-8111-111111111111"
internal const val COMMAND = "22222222-2222-4222-8222-222222222222"
internal const val INCARNATION = "33333333-3333-4333-8333-333333333333"
internal const val RUN = "44444444-4444-4444-8444-444444444444"
internal const val VESSEL = "55555555-5555-4555-8555-555555555555"
internal const val GRANT = "66666666-6666-4666-8666-666666666666"
internal const val TOKEN = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
internal fun fixture(name: String) = object {}.javaClass.getResource("/wire/$name.json")!!.readText().trim()
internal fun accepted() = VesselResponse.decode(wireJson.parseToJsonElement(fixture("accepted")).jsonObject)
internal fun endpoint() = VesselEndpoint("wss://vessel.example$SOCKET_PATH", VESSEL, GRANT, TOKEN)
internal fun mutation() = Mutation.submit(SESSION, COMMAND, 7, 1900000000000, "Hello π")
internal class Journal : MutationJournal {
    val entries = mutableMapOf<String, JournalEntry>()
    var failInsert = false
    var failRecord = false
    override suspend fun insert(entry: JournalEntry): Boolean {
        if (failInsert) throw VesselFailure("Storage unavailable")
        val old = entries[entry.commandId]
        if (old != null) {
            require(old.vesselId == entry.vesselId && old.sessionId == entry.sessionId && old.exactRequest == entry.exactRequest)
            return false
        }
        entries[entry.commandId] = entry
        return true
    }
    override suspend fun record(commandId: String, state: JournalState, responseJson: String?) {
        if (failRecord) throw VesselFailure("Storage unavailable")
        val old = entries.getValue(commandId)
        entries[commandId] = JournalEntry(old.vesselId, old.sessionId, commandId, old.exactRequest,
            if (old.state == JournalState.RESOLVED) old.state else state, responseJson)
    }
    override suspend fun pending(vesselId: String) = entries.values.filter { it.vesselId == vesselId && it.state == JournalState.UNCERTAIN }
}
internal class FakeTransport : VesselTransport {
    override val state = MutableStateFlow(ConnectionState(INCARNATION))
    val requests = mutableListOf<JsonObject>()
    var answer: suspend (JsonObject) -> VesselResponse = { accepted() }
    var closed = false
    override suspend fun connect() {}
    override suspend fun exchange(exactRequest: String): VesselResponse {
        val req = wireJson.parseToJsonElement(exactRequest).jsonObject
        requests.add(req)
        return answer(req)
    }
    override fun observe(subscriptions: List<EventSubscription>): Flow<VesselEvent> = emptyFlow()
    override fun close() { closed = true }
}

class ClientTest {
    @Test fun submitWireFixture() {
        assertEquals(wireJson.parseToJsonElement(fixture("submit")), mutation().request)
        assertEquals("Hello π", mutation().request["command"]!!.jsonObject.string("prompt"))
    }
    @Test fun publicOperationsAreFlattened() {
        val commands = listOf(Mutation.steer(SESSION, INCARNATION, COMMAND, 2, 1900000000000, RUN, "next"),
            Mutation.cancel(SESSION, INCARNATION, COMMAND, 2, 1900000000000, RUN),
            Mutation.respond(SESSION, INCARNATION, COMMAND, 2, 1900000000000, RUN, GRANT, JsonPrimitive("denied")))
        commands.forEach {
            val command = it.request["command"]!!.jsonObject
            assertEquals(INCARNATION, command.string("incarnation"))
            assertEquals(RUN, command.string("run_id"))
            assertFalse(command.containsKey("request"))
        }
        assertEquals(JsonNull, Requests.history(SESSION)["command"]!!.jsonObject["expected_revision"])
        assertFailsWith<IllegalArgumentException> { Requests.history(SESSION, limit = 129) }
        assertFailsWith<IllegalArgumentException> { Requests.events(SESSION, INCARNATION, -1) }
    }
    @Test fun uncertaintyDefaultsClosed() {
        val response = VesselResponse.decode(obj("protocol" to 1.json(), "result" to JsonNull, "error" to JsonNull))
        assertTrue(response.outcomeUnknown)
        assertFalse(response.successful)
    }
    @Test fun credentialsAndTextNeverInDiagnostics() {
        listOf(endpoint().toString(), mutation().toString(), accepted().toString(), JournalEntry(VESSEL, SESSION, COMMAND, fixture("submit")).toString()).forEach {
            assertFalse(it.contains(TOKEN)); assertFalse(it.contains("Hello")); assertFalse(it.contains("Bearer"))
        }
        assertFailsWith<IllegalArgumentException> { VesselEndpoint("ws://vessel.example$SOCKET_PATH", VESSEL, GRANT, TOKEN) }
        assertFailsWith<IllegalArgumentException> { VesselEndpoint("wss://user:secret@vessel.example$SOCKET_PATH", VESSEL, GRANT, TOKEN) }
        assertFailsWith<IllegalArgumentException> { VesselEndpoint("wss://vessel.example$SOCKET_PATH?token=$TOKEN", VESSEL, GRANT, TOKEN) }
        assertFailsWith<IllegalArgumentException> { VesselEndpoint("wss://vessel.example$SOCKET_PATH", VESSEL, GRANT, "invalid\nheader") }
    }
    @Test fun journalCommittedBeforeDispatchAndFailurePreventsSend() = runBlocking {
        val journal = Journal(); val transport = FakeTransport(); val client = VesselClient(endpoint(), journal, transport)
        transport.answer = { assertEquals(fixture("submit"), journal.entries.getValue(COMMAND).exactRequest); accepted() }
        client.mutate(mutation())
        assertEquals(JournalState.RESOLVED, journal.entries.getValue(COMMAND).state)
        assertEquals(1, transport.requests.size)
        val failed = Journal().apply { failInsert = true }; val untouched = FakeTransport()
        assertFailsWith<VesselFailure> { VesselClient(endpoint(), failed, untouched).mutate(mutation()) }
        assertTrue(untouched.requests.isEmpty())
    }
    @Test fun lostReplyAndProcessRestartUseOnlyReceipt() = runBlocking {
        val journal = Journal(); val first = FakeTransport().apply { answer = { throw VesselFailure("Disconnected") } }
        assertFailsWith<VesselFailure> { VesselClient(endpoint(), journal, first).mutate(mutation()) }
        assertEquals(JournalState.UNCERTAIN, journal.entries.getValue(COMMAND).state)
        val second = FakeTransport(); val client = VesselClient(endpoint(), journal, second)
        client.mutate(mutation())
        assertEquals(listOf("receipt"), second.requests.map { it["command"]!!.jsonObject.string("op") })
        assertEquals(JournalState.RESOLVED, journal.entries.getValue(COMMAND).state)
        assertEquals(fixture("submit"), journal.entries.getValue(COMMAND).exactRequest)
    }
    @Test fun sameIdentityDifferentPayloadNeverDispatches() = runBlocking {
        val journal = Journal(); val transport = FakeTransport(); val client = VesselClient(endpoint(), journal, transport)
        client.mutate(mutation())
        assertFailsWith<IllegalArgumentException> { client.mutate(Mutation.submit(SESSION, COMMAND, 7, 1900000000000, "different")) }
        assertEquals(1, transport.requests.size)
    }
    @Test fun unknownReceiptAndJournalUpdateFailureKeepUncertainty() = runBlocking {
        val journal = Journal().apply { failRecord = true }; val transport = FakeTransport(); val client = VesselClient(endpoint(), journal, transport)
        assertFailsWith<VesselFailure> { client.mutate(mutation()) }
        assertEquals(JournalState.UNCERTAIN, journal.entries.getValue(COMMAND).state)
        journal.failRecord = false
        transport.answer = { VesselResponse(obj("session_id" to SESSION.json(), "incarnation" to INCARNATION.json(), "result" to obj("command_id" to COMMAND.json(), "status" to "unknown".json())), null, false) }
        client.recoverPending()
        assertEquals(JournalState.UNCERTAIN, journal.entries.getValue(COMMAND).state)
        assertEquals("receipt", transport.requests.last()["command"]!!.jsonObject.string("op"))
    }
    @Test fun readCannotBypassJournalAndCloseNeverCancels() = runBlocking {
        val transport = FakeTransport(); val client = VesselClient(endpoint(), Journal(), transport)
        assertFailsWith<IllegalArgumentException> { client.read(mutation().request) }
        client.close()
        assertTrue(transport.closed); assertTrue(transport.requests.isEmpty())
    }
    @Test fun canonicalReconciliationPinsRevisionAndConnection(): Unit = runBlocking {
        val transport = FakeTransport(); val client = VesselClient(endpoint(), Journal(), transport)
        transport.answer = { req ->
            val command = req["command"]!!.jsonObject
            val result = if (command.string("op") == "snapshot") obj("session_id" to SESSION.json(), "revision" to 7.json(), "observation_cursor" to 12.json()) else {
                assertEquals(7L, command.number("expected_revision"))
                obj("session_id" to SESSION.json(), "revision" to 7.json(), "messages" to JsonArray(emptyList()))
            }
            VesselResponse(obj("session_id" to SESSION.json(), "incarnation" to INCARNATION.json(), "result" to result), null, false)
        }
        val reconciled = client.reconcile(SESSION)
        assertEquals(12L, reconciled.cursor)
        val prior = transport.answer
        transport.answer = { val response = prior(it); transport.state.value = ConnectionState(null, 1); response }
        assertFailsWith<IllegalArgumentException> { client.reconcile(SESSION) }
    }
    @Test fun mismatchedReceiptCannotResolveAnotherCommand() = runBlocking {
        val journal = Journal(); val transport = FakeTransport(); val client = VesselClient(endpoint(), journal, transport)
        journal.insert(JournalEntry(VESSEL, SESSION, COMMAND, fixture("submit")))
        transport.answer = { VesselResponse(obj("session_id" to SESSION.json(), "incarnation" to INCARNATION.json(), "result" to obj("command_id" to GRANT.json(), "status" to "accepted".json())), null, false) }
        client.recoverPending()
        assertEquals(JournalState.UNCERTAIN, journal.entries.getValue(COMMAND).state)
        assertEquals("receipt", transport.requests.single()["command"]!!.jsonObject.string("op"))
    }

    @Test fun jsonNestingBoundIgnoresEscapedStringBrackets() {
        assertFailsWith<IllegalArgumentException> { boundedFrame("[".repeat(65) + "0" + "]".repeat(65)) }
        val text = obj("text" to ("[".repeat(80) + "\\\"").json()).toString()
        assertEquals(wireJson.parseToJsonElement(text), boundedFrame(text))
    }

}
