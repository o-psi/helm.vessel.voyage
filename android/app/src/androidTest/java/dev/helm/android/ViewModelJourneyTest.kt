package dev.helm.android

import android.app.Application
import androidx.lifecycle.SavedStateHandle
import androidx.test.core.app.ApplicationProvider
import androidx.test.platform.app.InstrumentationRegistry
import dev.helm.vessel.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
import java.util.UUID

/** Android lifecycle + real Room/Keystore + client journal semantics. Synthetic transport, not TLS evidence. */
class ViewModelJourneyTest {
    @Test fun conversationMutationLossAndResumeReconcileWithoutReplay() = runBlocking {
        val app = ApplicationProvider.getApplicationContext<Application>()
        val vessel = UUID.randomUUID().toString()
        val session = UUID.randomUUID().toString()
        val incarnation = UUID.randomUUID().toString()
        val grant = UUID.randomUUID().toString()
        val store = CredentialStore(app)
        store.save("wss://test.example/v1/vessel/socket", vessel, grant, "c".repeat(64))
        val transports = mutableListOf<JourneyTransport>()
        lateinit var vm: HelmViewModel
        onMain {
            vm = HelmViewModel(app, SavedStateHandle())
            vm.clientFactory = { endpoint, journal ->
                val transport = JourneyTransport(session, incarnation)
                transports += transport
                VesselClient(endpoint, journal, transport)
            }
            vm.resume()
        }
        try {
            await { vm.state.value.voyages.isNotEmpty() }
            onMain { vm.select(session) }
            await { !vm.state.value.stale && vm.state.value.selected == session && vm.state.value.messages.isNotEmpty() }
            assertEquals("Canonical reply", vm.state.value.messages.single().text("content"))
            onMain { vm.draft("next prompt"); vm.send("submit") }
            await { vm.state.value.pending.isNotEmpty() }
            assertEquals(1, transports.sumOf { t -> t.ops.count { it == "submit" } })
            assertFalse(vm.state.value.actionable)
            onMain { vm.pause() }
            assertTrue(vm.state.value.stale)
            assertTrue(transports.first().closed)
            assertFalse(transports.flatMap { it.ops }.contains("cancel"))
            onMain { vm.resume() }
            await { vm.state.value.pending.isEmpty() && !vm.state.value.stale }
            assertEquals(1, transports.sumOf { t -> t.ops.count { it == "submit" } })
            assertTrue(transports.last().ops.contains("receipt"))
            assertEquals("next prompt", vm.draft.value) // lost reply never silently drops the draft
        } finally { onMain { vm.pause() }; store.forget() }
    }
    private fun onMain(action: () -> Unit) = InstrumentationRegistry.getInstrumentation().runOnMainSync(action)
    private suspend fun await(predicate: () -> Boolean) = withTimeout(15000) { while (!predicate()) delay(50) }
    private class JourneyTransport(val session: String, val incarnation: String) : VesselTransport {
        override val state = MutableStateFlow(ConnectionState())
        val ops = java.util.concurrent.CopyOnWriteArrayList<String>()
        var closed = false
        override suspend fun connect() { state.value = ConnectionState(UUID.randomUUID().toString()) }
        override fun close() { closed = true; state.value = ConnectionState(null, state.value.lossGeneration + 1) }
        override fun observe(subscriptions: List<EventSubscription>) = flow<VesselEvent> { awaitCancellation() }
        override suspend fun exchange(exactRequest: String): VesselResponse {
            val command = Json.parseToJsonElement(exactRequest).obj()["command"].obj()
            val op = command.text("op"); ops += op
            val message = buildJsonObject { put("message_index", 0); put("role", "assistant"); put("content", "Canonical reply") }
            if (op == "submit") { state.value = ConnectionState(null, 1); throw java.io.IOException("synthetic disconnect") }
            val result: JsonElement = when (op) {
                "catalogue" -> buildJsonArray { add(buildJsonObject { put("session_id", session); put("incarnation", incarnation); put("name", "Test voyage"); put("state", "suspended") }) }
                "snapshot" -> buildJsonObject { put("session_id", session); put("revision", 7); put("observation_cursor", 4); put("message_offset", 0); put("messages", buildJsonArray { add(message) }) }
                "history" -> buildJsonObject { put("session_id", session); put("revision", 7); put("message_offset", 0); put("messages", buildJsonArray { add(message) }); put("has_more", false); put("next_offset", 1) }
                "decisions" -> JsonArray(emptyList())
                "receipt" -> buildJsonObject { put("command_id", command.text("command_id")); put("status", "accepted") }
                else -> error("Unexpected request")
            }
            return VesselResponse(if (op == "catalogue") result else buildJsonObject { put("session_id", session); put("incarnation", incarnation); put("result", result) }, null, false)
        }
    }
}
