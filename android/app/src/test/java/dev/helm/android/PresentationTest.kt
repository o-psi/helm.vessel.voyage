package dev.helm.android
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
class PresentationTest {
    @Test fun staleAndUncertainCannotMutate() {
        val ready = HelmState(connected = true, stale = false)
        assertTrue(ready.actionable)
        assertFalse(ready.copy(stale = true).actionable)
        assertFalse(ready.copy(pending = listOf("intent")).actionable)
        assertFalse(ready.copy(busy = true).actionable)
        assertFalse(ready.copy(connected = false).actionable)
    }
    @Test fun stripsControlsWithoutRewritingCanonicalData() {
        assertEquals("hello\n世界", displayText("\u001bhello\n\u202e世界\u0000"))
    }
    @Test fun runningIsNotAdmissionOrCleanup() {
        assertTrue(HelmState(snapshot = buildJsonObject { put("run", buildJsonObject { put("state", "running") }) }).running)
        assertFalse(HelmState(snapshot = buildJsonObject { put("run", buildJsonObject { put("state", "completed") }); put("pending_cleanup_run", "id") }).running)
    }
}
