package dev.helm.vessel

import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import kotlin.test.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.*
import kotlinx.serialization.json.*
import okhttp3.*
import okhttp3.mockwebserver.*
import okhttp3.tls.HandshakeCertificates
import okhttp3.tls.HeldCertificate

class TransportTest {
    private class Peer(private val vessel: String = VESSEL, private val respond: Boolean = true) : AutoCloseable {
        val frames = LinkedBlockingQueue<JsonObject>()
        val certificate = HeldCertificate.Builder().commonName("localhost").addSubjectAlternativeName("localhost").build()
        val trust = HandshakeCertificates.Builder().addTrustedCertificate(certificate.certificate).build()
        val server = MockWebServer()
        lateinit var socket: WebSocket
        init {
            val tls = HandshakeCertificates.Builder().heldCertificate(certificate).build()
            server.useHttps(tls.sslSocketFactory(), false)
            server.enqueue(MockResponse().withWebSocketUpgrade(object : WebSocketListener() {
                override fun onOpen(webSocket: WebSocket, response: Response) {
                    socket = webSocket
                    webSocket.send(obj("type" to "hello".json(), "protocol" to 1.json(), "socket_id" to INCARNATION.json(), "vessel_id" to vessel.json()).toString())
                }
                override fun onMessage(webSocket: WebSocket, text: String) {
                    val frame = wireJson.parseToJsonElement(text).jsonObject
                    frames.add(frame)
                    when (frame.string("type")) {
                        "command" -> if (respond) webSocket.send(obj("type" to "reply".json(), "request_id" to frame.getValue("request_id"), "response" to wireJson.parseToJsonElement(fixture("accepted"))).toString())
                        "subscribe" -> {
                            webSocket.send(obj("type" to "subscribed".json(), "request_id" to frame.getValue("request_id")).toString())
                            webSocket.send(obj("type" to "event".json(), "subscription_id" to frame.getValue("request_id"), "event" to obj("protocol" to 1.json(), "session_id" to SESSION.json(), "incarnation" to INCARNATION.json(), "result" to obj("cursor" to 12.json(), "replay_gap" to JsonPrimitive(true)), "error" to JsonNull, "outcome_unknown" to JsonPrimitive(false))).toString())
                        }
                    }
                }
            }).setHeader("Sec-WebSocket-Protocol", SUBPROTOCOL))
            server.start()
        }
        fun transport(timeout: Long = 3000): OkHttpVesselTransport {
            val url = server.url(SOCKET_PATH).toString().replaceFirst("https://", "wss://")
            // Synthetic CA trusted only by test client; production uses platform TLS trust.
            val http = OkHttpClient.Builder().sslSocketFactory(trust.sslSocketFactory(), trust.trustManager)
                .retryOnConnectionFailure(false).followRedirects(false).build()
            return OkHttpVesselTransport(VesselEndpoint(url, VESSEL, GRANT, TOKEN), http, timeout)
        }
        override fun close() { if (::socket.isInitialized) socket.close(1000, null); server.close() }
    }
    @Test fun trustedWssHeadersAndSocketCorrelationDistinctFromDurableCommand() = runBlocking {
        Peer().use { peer ->
            val transport = peer.transport()
            try {
                transport.connect()
                assertEquals(INCARNATION, transport.state.value.socketId)
                val response = transport.exchange(mutation().request.toString())
                assertTrue(response.successful)
                val frame = peer.frames.poll(3, TimeUnit.SECONDS)!!
                assertEquals("command", frame.string("type"))
                assertNotEquals(COMMAND, frame.string("request_id"))
                assertEquals(mutation().request, frame["request"])
                val handshake = peer.server.takeRequest(3, TimeUnit.SECONDS)!!
                assertEquals("Bearer $TOKEN", handshake.getHeader("Authorization"))
                assertEquals(GRANT, handshake.getHeader("x-voyage-grant"))
                assertEquals(VESSEL, handshake.getHeader("x-voyage-vessel"))
                assertEquals(SUBPROTOCOL, handshake.getHeader("Sec-WebSocket-Protocol"))
            } finally { transport.close() }
            assertNull(peer.frames.poll(100, TimeUnit.MILLISECONDS)) // no cancel on close
        }
    }
    @Test fun identityMismatchRejectedBeforeCommand() = runBlocking {
        Peer(vessel = GRANT).use { peer ->
            val transport = peer.transport()
            try {
                assertFailsWith<VesselFailure> { transport.connect() }
                assertNull(transport.state.value.socketId)
                assertFailsWith<VesselFailure> { transport.exchange(Requests.catalogue().toString()) }
                assertTrue(peer.frames.isEmpty())
            } finally { transport.close() }
        }
    }
    @Test fun subscriptionInvalidationAndUnsubscribe() = runBlocking {
        Peer().use { peer ->
            val transport = peer.transport()
            try {
                transport.connect()
                val event = withTimeout(3000) { transport.observe(listOf(EventSubscription(SESSION, INCARNATION, 11))).first() }
                assertEquals(SESSION, event.sessionId)
                assertTrue(event.result.jsonObject["replay_gap"]!!.jsonPrimitive.boolean)
                val subscribe = peer.frames.poll(3, TimeUnit.SECONDS)!!
                val unsubscribe = peer.frames.poll(3, TimeUnit.SECONDS)!!
                assertEquals("subscribe", subscribe.string("type"))
                assertEquals("unsubscribe", unsubscribe.string("type"))
                assertEquals(subscribe["request_id"], unsubscribe["subscription_id"])
            } finally { transport.close() }
        }
    }
    @Test fun deadlineDisconnectsDoesNotReplayAndRetainsLossGeneration() = runBlocking {
        Peer(respond = false).use { peer ->
            val transport = peer.transport(250)
            try {
                transport.connect()
                assertFailsWith<TimeoutCancellationException> { transport.exchange(mutation().request.toString()) }
                assertNull(transport.state.value.socketId)
                assertTrue(transport.state.value.lossGeneration > 0)
                assertEquals(1, peer.frames.size)
                assertFailsWith<VesselFailure> { transport.exchange(Requests.catalogue().toString()) }
                assertEquals(1, peer.frames.size)
            } finally { transport.close() }
        }
    }
    @Test fun oversizedOutboundRejectedBeforeDispatch() = runBlocking {
        Peer().use { peer ->
            val transport = peer.transport()
            try {
                transport.connect()
                assertFailsWith<IllegalArgumentException> { transport.exchange(" ".repeat(MAX_FRAME_BYTES)) }
                assertTrue(peer.frames.isEmpty())
            } finally { transport.close() }
        }
    }
    @Test fun unsupportedReverseRequestCannotExecuteBrowserWork() = runBlocking {
        Peer().use { peer ->
            val transport = peer.transport()
            try {
                transport.connect()
                peer.socket.send(obj("type" to "reverse_request".json(), "request_id" to COMMAND.json(), "request" to obj("kind" to "browser_work".json(), "session_id" to SESSION.json(), "incarnation" to INCARNATION.json())).toString())
                val frame = withContext(Dispatchers.IO) { peer.frames.poll(3, TimeUnit.SECONDS)!! }
                assertEquals("reverse_reply", frame.string("type"))
                assertEquals("unavailable", frame["reply"]!!.jsonObject.string("status"))
            } finally { transport.close() }
        }
    }
}
