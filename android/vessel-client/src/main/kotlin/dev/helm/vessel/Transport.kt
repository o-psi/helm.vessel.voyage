package dev.helm.vessel

import java.io.Closeable
import java.util.UUID
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.*
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.*
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.serialization.json.*
import okhttp3.*
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okio.ByteString

/** Only trusted TLS + an existing scoped grant; no pairing, loopback token or TLS bypass. */
class VesselEndpoint(val wssUrl: String, val vesselId: String, val grantId: String, internal val token: String) {
    internal val httpUrl: HttpUrl
    init {
        uuid(vesselId); uuid(grantId)
        require(token.length == 64 && token.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }) { "Invalid scoped credential" }
        require(wssUrl.startsWith("wss://")) { "Trusted WSS required" }
        httpUrl = ("https://" + wssUrl.removePrefix("wss://")).toHttpUrlOrNull()
            ?: throw IllegalArgumentException("Invalid WSS endpoint")
        require(httpUrl.username.isEmpty() && httpUrl.password.isEmpty() && httpUrl.query == null && httpUrl.fragment == null && httpUrl.encodedPath == SOCKET_PATH) { "Expected credential-free Vessel socket URL" }
    }
    override fun toString() = "VesselEndpoint(vesselId=$vesselId, credentials=redacted)"
}

const val SOCKET_PATH = "/v1/vessel/socket"
const val SUBPROTOCOL = "voyage.vessel.v1"
const val MAX_FRAME_BYTES = 4 * 1024 * 1024

data class ConnectionState(val socketId: String? = null, val lossGeneration: Long = 0)
data class EventSubscription(val sessionId: String, val incarnation: String, val after: Long) {
    init { uuid(sessionId); uuid(incarnation); require(after >= 0) }
    internal fun json() = obj("session_id" to sessionId.json(), "incarnation" to incarnation.json(), "after" to after.json())
}
class VesselEvent(val sessionId: String, val incarnation: String, val result: JsonElement) {
    internal var queuedBytes: Int = 0
    override fun toString() = "VesselEvent(sessionId=$sessionId)"
}

/** Injectable boundary for deterministic journal tests. Never automatically retries commands. */
interface VesselTransport : Closeable {
    val state: StateFlow<ConnectionState>
    suspend fun connect()
    suspend fun exchange(exactRequest: String): VesselResponse
    fun observe(subscriptions: List<EventSubscription>): Flow<VesselEvent>
}

/** One activation, reconnect only through explicit connect(). Loss fences all pending work. */
class OkHttpVesselTransport internal constructor(
    private val endpoint: VesselEndpoint,
    private val http: OkHttpClient,
    private val timeoutMs: Long,
) : VesselTransport {
    constructor(endpoint: VesselEndpoint) : this(endpoint, OkHttpClient.Builder()
        .connectTimeout(8, TimeUnit.SECONDS).readTimeout(15, TimeUnit.SECONDS)
        .pingInterval(5, TimeUnit.SECONDS).retryOnConnectionFailure(false)
        .followRedirects(false).followSslRedirects(false).build(), 15_000)

    private val monitor = Any()
    private val connecting = Mutex()
    private val mutableState = MutableStateFlow(ConnectionState())
    override val state: StateFlow<ConnectionState> = mutableState.asStateFlow()
    private var active: Socket? = null
    private var closed = false
    private class Socket {
        var webSocket: WebSocket? = null
        var opened = false
        var ready = false
        var failed = false
        var eventBytes = 0
        val hello = CompletableDeferred<Unit>()
        val pending = mutableMapOf<String, CompletableDeferred<JsonObject>>()
        val subscriptions = mutableMapOf<String, Pair<Map<String, String>, Channel<VesselEvent>>>()
    }
    override suspend fun connect() = connecting.withLock {
        val socket = synchronized(monitor) {
            check(!closed) { "Connection activation closed" }
            if (active?.ready == true) return@withLock
            Socket().also { active = it }
        }
        val request = Request.Builder().url(endpoint.httpUrl)
            .header("Authorization", "Bearer ${endpoint.token}")
            .header("x-voyage-grant", endpoint.grantId)
            .header("x-voyage-vessel", endpoint.vesselId)
            .header("Sec-WebSocket-Protocol", SUBPROTOCOL).build()
        try {
            val ws = http.newWebSocket(request, listener(socket))
            synchronized(monitor) {
                socket.webSocket = ws
                if (socket.failed) ws.cancel()
            }
            withTimeout(timeoutMs) { socket.hello.await() }
        } catch (failure: Exception) {
            fail(socket)
            if (failure is CancellationException) throw failure
            throw VesselFailure("Vessel connection unavailable")
        }
    }
    private fun listener(socket: Socket) = object : WebSocketListener() {
        override fun onOpen(webSocket: WebSocket, response: Response) {
            synchronized(monitor) {
                if (socket.failed || active !== socket || response.header("Sec-WebSocket-Protocol") != SUBPROTOCOL) {
                    webSocket.cancel(); fail(socket); return
                }
                socket.webSocket = webSocket
                socket.opened = true
            }
        }
        override fun onMessage(webSocket: WebSocket, text: String) {
            try {
                if (text.toByteArray(Charsets.UTF_8).size > MAX_FRAME_BYTES) throw VesselFailure("Frame limit")
                val frame = boundedFrame(text)
                synchronized(monitor) {
                    if (socket.failed || active !== socket) return
                    val type = frame.string("type")
                    if (!socket.ready) {
                        require(socket.opened && type == "hello" && frame.number("protocol") == 1L && frame.string("vessel_id") == endpoint.vesselId)
                        val socketId = uuid(frame.string("socket_id"))
                        socket.ready = true
                        mutableState.value = mutableState.value.copy(socketId = socketId)
                        socket.hello.complete(Unit)
                        return
                    }
                    when (type) {
                        "reply", "subscribed" -> socket.pending.remove(frame.string("request_id"))?.complete(frame)
                            ?: throw VesselFailure("Unknown correlation")
                        "event" -> {
                            val subscription = socket.subscriptions[frame.string("subscription_id")] ?: return
                            val event = frame.getValue("event").jsonObject
                            require(event.number("protocol") == 1L && event["error"] == JsonNull && event["outcome_unknown"]?.jsonPrimitive?.boolean == false)
                            val id = event.string("session_id")
                            require(subscription.first[id] == event.string("incarnation"))
                            val bytes = text.toByteArray(Charsets.UTF_8).size
                            require(socket.eventBytes + bytes <= MAX_FRAME_BYTES)
                            val incoming = VesselEvent(id, event.string("incarnation"), event.getValue("result")).also { it.queuedBytes = bytes }
                            if (!subscription.second.trySend(incoming).isSuccess) throw VesselFailure("Event queue full")
                            socket.eventBytes += bytes
                        }
                        "reverse_request" -> send(socket, obj("type" to "reverse_reply".json(), "request_id" to uuid(frame.string("request_id")).json(), "reply" to obj("status" to "unavailable".json())).toString())
                        else -> throw VesselFailure("Unexpected frame")
                    }
                }
            } catch (_: Exception) { fail(socket) }
        }
        override fun onMessage(webSocket: WebSocket, bytes: ByteString) { fail(socket) }
        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) { fail(socket) }
        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) { fail(socket) }
        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) { fail(socket) }
    }
    private fun fail(socket: Socket) = synchronized(monitor) {
        if (socket.failed) return@synchronized
        socket.failed = true
        socket.ready = false
        socket.webSocket?.cancel()
        val failure = VesselFailure("Vessel disconnected; outcome unknown; recover receipts without replay")
        socket.hello.completeExceptionally(failure)
        socket.pending.values.forEach { it.completeExceptionally(failure) }
        socket.pending.clear()
        socket.subscriptions.values.forEach { it.second.close(failure) }
        socket.subscriptions.clear()
        if (active === socket) {
            mutableState.value = ConnectionState(null, mutableState.value.lossGeneration + 1)
            active = null
        }
    }
    private fun send(socket: Socket, text: String) {
        val bytes = text.toByteArray(Charsets.UTF_8).size
        val ws = socket.webSocket
        if (bytes > MAX_FRAME_BYTES || ws == null || ws.queueSize() + bytes > MAX_FRAME_BYTES || !ws.send(text)) throw VesselFailure("Vessel send unavailable")
    }
    override suspend fun exchange(exactRequest: String): VesselResponse {
        // Parse before adding any socket envelope; preserve exact serialized request bytes on dispatch.
        require(exactRequest.toByteArray(Charsets.UTF_8).size <= MAX_FRAME_BYTES - 128) { "Request limit" }
        val id = UUID.randomUUID().toString()
        val reply = CompletableDeferred<JsonObject>()
        val socket = synchronized(monitor) {
            val s = active?.takeIf { it.ready } ?: throw VesselFailure("Vessel not connected")
            if (s.pending.size >= 32) throw VesselFailure("Vessel request capacity reached")
            s.pending[id] = reply
            try { send(s, "{\"type\":\"command\",\"request_id\":\"$id\",\"request\":$exactRequest}") }
            catch (e: Exception) { fail(s); throw e }
            s
        }
        try {
            val frame = withTimeout(timeoutMs) { reply.await() }
            require(frame.string("type") == "reply")
            return VesselResponse.decode(frame.getValue("response").jsonObject)
        } catch (failure: Exception) {
            fail(socket)
            if (failure is CancellationException) throw failure
            throw VesselFailure("Vessel response unavailable; retain command identity")
        }
    }
    override fun observe(subscriptions: List<EventSubscription>): Flow<VesselEvent> = flow {
        require(subscriptions.size in 1..256 && subscriptions.map { it.sessionId }.distinct().size == subscriptions.size)
        val id = UUID.randomUUID().toString()
        val reply = CompletableDeferred<JsonObject>()
        // Small bounded invalidation queue; overflow disconnects rather than silently losing events.
        val events = Channel<VesselEvent>(8)
        val socket = synchronized(monitor) {
            val s = active?.takeIf { it.ready } ?: throw VesselFailure("Vessel not connected")
            if (s.pending.size >= 32 || s.subscriptions.size >= 32) throw VesselFailure("Vessel subscription capacity reached")
            s.pending[id] = reply
            s.subscriptions[id] = subscriptions.associate { it.sessionId to it.incarnation } to events
            try { send(s, obj("type" to "subscribe".json(), "request_id" to id.json(), "request" to obj("protocol" to 1.json(), "subscriptions" to JsonArray(subscriptions.map { it.json() }))).toString()) }
            catch (e: Exception) { fail(s); throw e }
            s
        }
        try {
            require(withTimeout(timeoutMs) { reply.await() }.string("type") == "subscribed")
            for (event in events) {
                synchronized(monitor) { socket.eventBytes -= event.queuedBytes }
                emit(event)
            }
        } finally {
            synchronized(monitor) {
                socket.subscriptions.remove(id)
                while (true) {
                    val discarded = events.tryReceive().getOrNull() ?: break
                    socket.eventBytes -= discarded.queuedBytes
                }
                events.cancel()
                if (socket.pending.remove(id) != null) fail(socket)
                else if (socket.ready) try { send(socket, obj("type" to "unsubscribe".json(), "subscription_id" to id.json()).toString()) } catch (_: Exception) { fail(socket) }
            }
        }
    }
    override fun close() = synchronized(monitor) {
        closed = true
        active?.let(::fail)
        // Only this client's socket is closed. Never submit a remote cancel/stop.
        Unit
    }
}

/** Limit JSON nesting before recursive tree decoding; strings may contain arbitrary brackets. */
internal fun boundedFrame(text: String): JsonObject {
    var depth = 0
    var quoted = false
    var escaped = false
    for (char in text) {
        if (quoted) {
            if (escaped) escaped = false
            else if (char == '\\') escaped = true
            else if (char == '"') quoted = false
        } else when (char) {
            '"' -> quoted = true
            '{', '[' -> { depth++; require(depth <= 64) { "Frame nesting limit" } }
            '}', ']' -> depth--
        }
    }
    return wireJson.parseToJsonElement(text).jsonObject
}
