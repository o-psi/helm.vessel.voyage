package dev.helm.android

import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.test.*
import androidx.compose.ui.test.junit4.createComposeRule
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test

class ConversationUiTest {
    @get:Rule val compose = createComposeRule()
    @Test fun voyageSelectionAndApprovalRequireExplicitNativeAction() {
        var selected = ""
        val voyage = buildJsonObject { put("session_id", "test-voyage"); put("name", "Work voyage"); put("state", "live") }
        compose.setContent { MaterialTheme { VoyageList(HelmState(voyages = listOf(voyage)), { selected = it }) } }
        compose.onNodeWithText("Work voyage").performClick()
        assertEquals("test-voyage", selected)
    }
    @Test fun approvalConfirmationDoesNotSendOnFirstTap() {
        var answer: JsonElement? = null
        val decision = buildJsonObject {
            put("decision_id", "approval"); put("expires_at_ms", System.currentTimeMillis() + 60000)
            put("request", buildJsonObject { put("kind", "approval"); put("approval", buildJsonObject { put("action", "Read file"); put("target", "example.txt"); put("reason", "Explicit request") }) })
        }
        compose.setContent { MaterialTheme { DecisionCard(decision, true) { _, value -> answer = value } } }
        compose.onNodeWithText("Approve…").performClick()
        assertNull(answer)
        compose.onNodeWithText("Approve operation").performClick()
        assertEquals(JsonPrimitive("approved"), answer)
    }
    @Test fun canonicalConversationKeepsProvisionalSuffixSeparateAndExpandsExplicitly() {
        var expanded = -1L
        val message = buildJsonObject { put("message_index", 3); put("role", "assistant"); put("content", "Canonical message"); put("projection_truncated", true) }
        val snapshot = buildJsonObject { put("name", "Example voyage"); put("run", buildJsonObject { put("state", "running"); put("stream_reconciled", true); put("live_text", "Streaming suffix") }) }
        compose.setContent { MaterialTheme { Conversation(HelmState(connected = true, stale = false, selected = "example", snapshot = snapshot, messages = listOf(message), decisionAvailable = true), {}, { expanded = it }, { _, _ -> }) } }
        compose.onNodeWithText("Canonical message").assertIsDisplayed()
        compose.onNodeWithText("Live · provisional").assertIsDisplayed()
        compose.onNodeWithText("Streaming suffix").assertIsDisplayed()
        compose.onNodeWithText("Load full message (up to 4 MiB)").performClick()
        assertEquals(3L, expanded)
    }
    @Test fun staleDecisionCannotGrantApproval() {
        var answered = false
        val decision = buildJsonObject {
            put("decision_id", "approval"); put("expires_at_ms", System.currentTimeMillis() + 60000)
            put("request", buildJsonObject { put("kind", "approval"); put("approval", buildJsonObject { put("action", "Read file") }) })
        }
        compose.setContent { MaterialTheme { DecisionCard(decision, false) { _, _ -> answered = true } } }
        compose.onNodeWithText("Approve…").assertIsNotEnabled()
        compose.onNodeWithText("Deny").assertIsNotEnabled()
        assertFalse(answered)
    }
}
