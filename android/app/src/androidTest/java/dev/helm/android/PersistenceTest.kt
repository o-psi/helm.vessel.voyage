package dev.helm.android

import androidx.test.core.app.ApplicationProvider
import android.content.Context
import dev.helm.vessel.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
import java.util.UUID

class PersistenceTest {
    private val context get() = ApplicationProvider.getApplicationContext<Context>()
    private val vessel = "11111111-1111-4111-8111-111111111111"
    private val session = "22222222-2222-4222-8222-222222222222"
    @Test fun durableJournalSurvivesReopenAndRejectsReplacement() = runBlocking {
        val command = UUID.randomUUID().toString()
        val entry = JournalEntry(vessel, session, command, "exact payload")
        var db = HelmDatabase.open(context)
        var journal = RoomJournal(db)
        assertTrue(journal.insert(entry)); assertFalse(journal.insert(entry))
        assertTrue(runCatching { journal.insert(JournalEntry(vessel, session, command, "different")) }.isFailure)
        db.close()
        db = HelmDatabase.open(context); journal = RoomJournal(db)
        assertTrue(journal.pending(vessel).any { it.commandId == command && it.exactRequest == "exact payload" })
        journal.record(command, JournalState.RESOLVED, "known admission")
        journal.record(command, JournalState.UNCERTAIN, "late failure")
        assertEquals("RESOLVED", db.dao().mutation(command)?.state)
        assertEquals("known admission", db.dao().mutation(command)?.responseJson)
        db.close()
    }
    @Test fun concurrentIdentityHasOnlyOneInsert() = runBlocking {
        val db = HelmDatabase.open(context); val journal = RoomJournal(db)
        val entry = JournalEntry(vessel, session, UUID.randomUUID().toString(), "same")
        val results = (1..16).map { async(Dispatchers.IO) { journal.insert(entry) } }.awaitAll()
        assertEquals(1, results.count { it }); db.close()
    }
    @Test fun keystoreCiphertextRoundTripAndTamperFailsClosed() {
        val store = CredentialStore(context)
        val token = "a".repeat(64)
        store.save("wss://test.example/v1/vessel/socket", vessel, session, token)
        val prefs = context.getSharedPreferences("connection", Context.MODE_PRIVATE)
        val encrypted = requireNotNull(prefs.getString("encrypted", null))
        assertFalse(encrypted.contains(token))
        assertEquals(token, CredentialStore(context).load()?.text("token"))
        prefs.edit().putString("encrypted", "invalid").commit()
        assertTrue(runCatching { store.load() }.isFailure)
        store.forget(); assertNull(store.load())
    }
    @Test fun clientWithRoomRecoversLostReplyWithoutResending() = runBlocking {
        val db = HelmDatabase.open(context); val journal = RoomJournal(db)
        val transport = RecordingTransport(session)
        val endpoint = VesselEndpoint("wss://test.example/v1/vessel/socket", vessel, session, "b".repeat(64))
        val mutation = Mutation.submit(session, UUID.randomUUID().toString(), 1, System.currentTimeMillis() + 60000, "synthetic prompt")
        val first = VesselClient(endpoint, journal, transport)
        assertTrue(runCatching { first.mutate(mutation) }.isFailure)
        assertEquals(1, transport.ops.count { it == "submit" }); first.close(); db.close()
        val reopened = HelmDatabase.open(context)
        val second = VesselClient(endpoint, RoomJournal(reopened), transport)
        second.mutate(mutation)
        assertEquals(listOf("submit", "receipt"), transport.ops)
        assertTrue(RoomJournal(reopened).pending(vessel).none { it.commandId == mutation.commandId })
        second.close(); reopened.close()
    }
    private class RecordingTransport(val session: String) : VesselTransport {
        override val state = MutableStateFlow(ConnectionState("33333333-3333-4333-8333-333333333333"))
        val ops = mutableListOf<String>()
        override suspend fun connect() {}
        override fun close() {}
        override fun observe(subscriptions: List<EventSubscription>) = emptyFlow<VesselEvent>()
        override suspend fun exchange(exactRequest: String): VesselResponse {
            val op = Json.parseToJsonElement(exactRequest).obj()["command"].obj().text("op"); ops += op
            if (op == "submit") throw java.io.IOException("synthetic lost reply")
            return VesselResponse(buildJsonObject {
                put("session_id", session); put("incarnation", "44444444-4444-4444-8444-444444444444")
                put("result", buildJsonObject { put("status", "accepted"); put("command_id", Json.parseToJsonElement(exactRequest).obj()["command"].obj().text("command_id")) })
            }, null, false)
        }
    }
}
