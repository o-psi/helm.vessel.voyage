package dev.helm.android

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import dev.helm.vessel.*
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.FixMethodOrder
import org.junit.Test
import org.junit.runners.MethodSorters
import java.util.UUID

/** Also invoked as two separate instrumentation runs with adb force-stop between them. */
@FixMethodOrder(MethodSorters.NAME_ASCENDING)
class ProcessDeathTest {
    private val context get() = ApplicationProvider.getApplicationContext<Context>()
    private val vessel = "88888888-8888-4888-8888-888888888888"
    private val session = "99999999-9999-4999-8999-999999999999"
    @Test fun aSeed() = runBlocking {
        val id = UUID.randomUUID().toString()
        val mutation = Mutation.submit(session, id, 7, System.currentTimeMillis() + 60000, "process-death synthetic intent")
        val db = HelmDatabase.open(context)
        assertTrue(RoomJournal(db).insert(JournalEntry(vessel, session, id, mutation.request.toString())))
        db.dao().cache(CacheRow(vessel, "process-death", "canonical cache", 123))
        assertTrue(context.getSharedPreferences("death-fixture", Context.MODE_PRIVATE).edit().putString("command", id).commit())
        CredentialStore(context).save("wss://test.example/v1/vessel/socket", vessel, session, "d".repeat(64))
        db.close()
    }
    @Test fun bRecover() = runBlocking {
        val id = requireNotNull(context.getSharedPreferences("death-fixture", Context.MODE_PRIVATE).getString("command", null))
        val db = HelmDatabase.open(context)
        val entry = requireNotNull(db.dao().mutation(id))
        assertEquals("UNCERTAIN", entry.state)
        assertTrue(entry.exactRequest.contains("process-death synthetic intent"))
        assertEquals("canonical cache", db.dao().cached(vessel, "process-death")?.json)
        assertEquals("d".repeat(64), CredentialStore(context).load()?.text("token"))
        // No dispatch here. The separate client+Room test establishes recovery emits only receipt.
        db.close(); CredentialStore(context).forget()
    }
}
