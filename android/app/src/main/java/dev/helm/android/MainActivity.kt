package dev.helm.android

import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.*
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import kotlinx.serialization.json.*

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // Conversation and credentials stay out of screenshots and the recents preview.
        window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        enableEdgeToEdge()
        setContent { MaterialTheme { HelmApp() } }
    }
}
@Composable
fun HelmApp(vm: HelmViewModel = viewModel()) {
    val state by vm.state.collectAsStateWithLifecycle()
    val draft by vm.draft.collectAsStateWithLifecycle()
    var setup by rememberSaveable { mutableStateOf(false) }
    var confirmCancel by remember { mutableStateOf(false) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(lifecycle, vm) {
        val observer = LifecycleEventObserver { _, event ->
            when (event) { Lifecycle.Event.ON_START -> vm.resume(); Lifecycle.Event.ON_STOP -> vm.pause(); else -> Unit }
        }
        lifecycle.addObserver(observer)
        if (lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) vm.resume()
        onDispose { lifecycle.removeObserver(observer); vm.pause() }
    }
    BackHandler(setup || state.selected != null) { if (setup) setup = false else vm.select(null) }
    Surface(Modifier.fillMaxSize()) {
        Column(Modifier.safeDrawingPadding().imePadding().padding(horizontal = 16.dp)) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                TextButton(onClick = { if (setup) setup = false else vm.select(null) }) { Text(if (state.selected != null || setup) "‹ Voyages" else "Helm") }
                TextButton(onClick = { setup = !setup }) { Text("Connection") }
            }
            Text(state.notice, style = MaterialTheme.typography.bodySmall, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
            if (state.stale && state.configured) Text("STALE · Saved content is not a current observation", color = MaterialTheme.colorScheme.error)
            if (state.busy) LinearProgressIndicator(Modifier.fillMaxWidth())
            if (setup || !state.configured) {
                ConnectionSetup(state, onConnect = { url, vessel, grant, token -> vm.configure(url, vessel, grant, token); setup = false }, onForget = vm::forget)
            } else {
                Row {
                    TextButton(onClick = vm::reconnect) { Text(if (state.connected) "Reconnect" else "Connect") }
                    TextButton(onClick = vm::refresh, enabled = state.connected) { Text("Refresh") }
                    TextButton(onClick = vm::pause) { Text("Disconnect") }
                }
                if (state.selected == null) VoyageList(state, vm::select, Modifier.weight(1f)) else {
                    if (state.actionStatus.isNotBlank()) Text(state.actionStatus, style = MaterialTheme.typography.bodySmall, modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite })
                    if (state.pending.isNotEmpty()) {
                        Text("${state.pending.size} uncertain command(s). No automatic resend.", color = MaterialTheme.colorScheme.error)
                        TextButton(onClick = vm::recoverReceipts, enabled = state.connected) { Text("Check receipts") }
                    }
                    Conversation(state, vm::earlier, vm::expand, { decision, answer -> vm.send("respond", decision, answer) }, Modifier.weight(1f))
                    OutlinedTextField(value = draft, onValueChange = vm::draft, label = { Text("Message (not credentials)") }, modifier = Modifier.fillMaxWidth(), maxLines = 5)
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        Button(onClick = { vm.send("submit") }, enabled = state.actionable && !state.running && draft.isNotBlank()) { Text("Send") }
                        OutlinedButton(onClick = { vm.send("steer") }, enabled = state.actionable && state.running && draft.isNotBlank()) { Text("Steer") }
                        TextButton(onClick = { confirmCancel = true }, enabled = state.actionable && state.running) { Text("Cancel run") }
                    }
                }
            }
        }
    }
    if (confirmCancel) AlertDialog(onDismissRequest = { confirmCancel = false }, title = { Text("Cancel this remote run?") },
        text = { Text("This requests cancellation on Vessel. Cleanup may remain pending. Disconnecting instead leaves execution running.") },
        confirmButton = { TextButton(onClick = { confirmCancel = false; vm.send("cancel") }) { Text("Request cancellation") } },
        dismissButton = { TextButton(onClick = { confirmCancel = false }) { Text("Keep running") } })
}

@Composable
private fun ConnectionSetup(state: HelmState, onConnect: (String, String, String, String) -> Unit, onForget: () -> Unit) {
    // Not rememberSaveable: no grant secret in Android saved-instance bundles or ViewModel state.
    var url by remember { mutableStateOf("") }
    var vessel by remember { mutableStateOf("") }
    var grant by remember { mutableStateOf("") }
    var token by remember { mutableStateOf("") }
    Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Text("Connect directly to Vessel", style = MaterialTheme.typography.headlineSmall)
        Text("Use an existing scoped grant and a WSS endpoint with a trusted certificate. Obtain the Vessel identity independently. Never enter provider credentials or a local loopback token.")
        OutlinedTextField(url, { url = it }, label = { Text("wss://host/v1/vessel/socket") }, singleLine = true, modifier = Modifier.fillMaxWidth(), keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri))
        OutlinedTextField(vessel, { vessel = it }, label = { Text("Expected Vessel UUID") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(grant, { grant = it }, label = { Text("Scoped grant UUID") }, singleLine = true, modifier = Modifier.fillMaxWidth())
        OutlinedTextField(token, { token = it }, label = { Text("Scoped grant token") }, singleLine = true, modifier = Modifier.fillMaxWidth(), visualTransformation = PasswordVisualTransformation(), keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, autoCorrectEnabled = false))
        Text("Saved with Android Keystore encryption; backups and screen capture are disabled. Connection setup does not expand the grant's authority.", style = MaterialTheme.typography.bodySmall)
        Button(onClick = { onConnect(url, vessel, grant, token); token = "" }, enabled = listOf(url, vessel, grant, token).all { it.isNotBlank() }) { Text("Save and connect") }
        if (state.configured) TextButton(onClick = { token = ""; onForget() }) { Text("Forget connection credentials") }
        Text("Forgetting credentials retains the private command journal and cache, so reconnecting to the same Vessel can recover uncertain commands.")
    }
}
@Composable
internal fun VoyageList(state: HelmState, select: (String) -> Unit, modifier: Modifier = Modifier) {
    LazyColumn(modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        if (state.voyages.isEmpty()) item { Text("No visible voyages. The grant may not authorize catalogue access, or no voyages exist.") }
        items(state.voyages, key = { it.text("session_id") }) { voyage ->
            OutlinedCard(onClick = { select(voyage.text("session_id")) }, modifier = Modifier.fillMaxWidth()) {
                Column(Modifier.padding(16.dp)) {
                    Text(displayText(voyage.text("name").ifBlank { voyage.text("session_id") }), style = MaterialTheme.typography.titleMedium)
                    Text(displayText(voyage.text("state")))
                    Text(displayText(voyage.text("workspace")), style = MaterialTheme.typography.bodySmall)
                }
            }
        }
    }
}
@Composable
internal fun Conversation(state: HelmState, earlier: () -> Unit, expand: (Long) -> Unit, answer: (JsonObject, JsonElement) -> Unit, modifier: Modifier = Modifier) {
    LazyColumn(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(12.dp)) {
        item {
            Text(displayText(state.snapshot.text("name").ifBlank { state.selected.orEmpty() }), style = MaterialTheme.typography.titleMedium)
            Text("Run: ${state.run.text("state").ifBlank { "none observed" }}")
            if (state.snapshot["cleanup"] != null && state.snapshot["cleanup"] != JsonNull) Text("Cleanup has recorded state; cancellation is not proof of completion.")
            state.snapshot.text("recovery_notice").takeIf { it.isNotBlank() }?.let { Text(displayText(it)) }
            if (state.earlierOffset > 0) TextButton(onClick = earlier, enabled = state.connected && !state.stale) { Text("Load earlier messages") }
        }
        items(state.messages, key = { it.number("message_index") }) { message ->
            Column {
                Text(message.text("role"), style = MaterialTheme.typography.labelMedium)
                SelectionContainer { Text(displayText(message.text("content"))) }
                if (message.flag("projection_truncated")) TextButton(onClick = { expand(message.number("message_index")) }, enabled = state.connected && !state.stale) { Text("Load full message (up to 4 MiB)") }
                if (message["tool_calls"].arr().isNotEmpty()) Text("Tool calls: " + message["tool_calls"].arr().joinToString { it.obj().text("name") })
            }
        }
        if (state.run.flag("stream_reconciled") && state.run.text("live_text").isNotEmpty()) item {
            Text("Live · provisional", style = MaterialTheme.typography.labelMedium)
            SelectionContainer { Text(displayText(state.run.text("live_text"))) }
            if (state.run.flag("live_text_truncated")) Text("Live preview truncated; canonical history will reconcile on refresh.")
        }
        if (!state.run.flag("stream_reconciled") && state.run.text("run_id").isNotEmpty()) item { Text("Live stream cannot yet be reconciled. Showing canonical history only.") }
        state.run.text("failure_summary").takeIf { it.isNotBlank() }?.let { summary -> item { Text(displayText(summary), color = MaterialTheme.colorScheme.error) } }
        items(state.decisions, key = { it.text("decision_id") }) { decision -> DecisionCard(decision, state.actionable, answer) }
        if (!state.decisionAvailable) item { Text("Decision controls unavailable or not authorized by this grant.", style = MaterialTheme.typography.bodySmall) }
    }
}
@Composable
internal fun DecisionCard(decision: JsonObject, enabled: Boolean, answer: (JsonObject, JsonElement) -> Unit) {
    val request = decision["request"].obj()
    val available = enabled && decision.number("expires_at_ms") > System.currentTimeMillis()
    var confirm by remember { mutableStateOf<JsonElement?>(null) }
    var custom by remember { mutableStateOf("") }
    OutlinedCard(Modifier.fillMaxWidth()) {
        Column(Modifier.padding(12.dp)) {
            when (request.text("kind")) {
                "approval" -> {
                    val approval = request["approval"].obj()
                    Text("Approval required", style = MaterialTheme.typography.titleMedium)
                    Text(displayText("${approval.text("action")}\n${approval.text("target")}\n${approval.text("reason")}"))
                    Row {
                        Button(onClick = { confirm = JsonPrimitive("approved") }, enabled = available) { Text("Approve…") }
                        TextButton(onClick = { answer(decision, JsonPrimitive("denied")) }, enabled = available) { Text("Deny") }
                    }
                }
                "question" -> {
                    val question = request["question"].obj()
                    Text(displayText(question.text("question")))
                    question["options"].arr().forEachIndexed { index, option ->
                        val text = option.jsonPrimitive.content
                        TextButton(onClick = { answer(decision, buildJsonObject { put("status", "selected"); put("index", index); put("answer", text) }) }, enabled = available) { Text(displayText(text)) }
                    }
                    OutlinedTextField(custom, { custom = it }, label = { Text("Custom answer (never secrets)") }, modifier = Modifier.fillMaxWidth())
                    TextButton(onClick = { answer(decision, buildJsonObject { put("status", "custom"); put("answer", custom) }); custom = "" }, enabled = available && custom.isNotBlank()) { Text("Answer") }
                    TextButton(onClick = { answer(decision, buildJsonObject { put("status", "cancelled") }) }, enabled = available) { Text("Dismiss question") }
                }
                else -> Text("Unsupported decision type. Use another authorized Helm client; no response is invented.")
            }
            if (!available) Text("Decision expired, stale, or awaiting reconciliation.")
        }
    }
    if (confirm != null) AlertDialog(onDismissRequest = { confirm = null }, title = { Text("Approve this exact operation?") }, text = { val approval = request["approval"].obj(); Text(displayText("${approval.text("action")}\n${approval.text("target")}\n${approval.text("reason")}\n\nApproval is sent under the executing voyage’s current policy.")) },
        confirmButton = { TextButton(onClick = { confirm?.let { answer(decision, it) }; confirm = null }, enabled = available) { Text("Approve operation") } },
        dismissButton = { TextButton(onClick = { confirm = null }) { Text("Back") } })
}
