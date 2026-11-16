package com.aspen_chat.client.call

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.os.Build
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.aspen_chat.client.push.PushHandler
import com.aspen_chat.client.push.PushStore
import com.aspen_chat.client.push.WebPush
import java.io.BufferedReader
import java.io.InputStream
import java.io.InputStreamReader
import java.io.OutputStream
import java.math.BigInteger
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.nio.charset.StandardCharsets
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.MessageDigest
import java.security.SecureRandom
import java.security.interfaces.ECPrivateKey
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.security.spec.ECPublicKeySpec
import java.time.Instant
import java.util.Base64
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import javax.crypto.Cipher
import javax.crypto.KeyAgreement
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * A DM's call ringing the phone, from the push to the ring's end, on the emulator: a `call` pointer
 * encrypted as a deployment encrypts it is handed to the push handler, which within five seconds
 * has the call in Telecom as incoming with the incoming call's notification posted; the engine's
 * ring watch identifies on a stand-in deployment's event stream, which then says the ring was
 * declined elsewhere, and the call and its notification go. A push that cannot be read rings
 * nothing. Answering from the lock screen and the ringtone are device checks (`android-builds.md`,
 * Rings).
 */
@RunWith(AndroidJUnit4::class)
class CallRingsTest {
    private lateinit var context: Context
    private lateinit var notifications: NotificationManager
    private lateinit var server: ServerSocket
    private lateinit var origin: String
    private lateinit var phonePublic: ByteArray
    private lateinit var auth: ByteArray

    /** The event stream connections the stand-in deployment took, each with what it was sent. */
    private val identified = LinkedBlockingQueue<Stream>()

    /** One event stream connection: frames go out through `send`. */
    private class Stream(val identify: JSONObject, private val out: OutputStream) {
        fun send(frame: JSONObject) {
            val payload = frame.toString().toByteArray(StandardCharsets.UTF_8)
            val header = if (payload.size < 126) {
                byteArrayOf(0x81.toByte(), payload.size.toByte())
            } else {
                byteArrayOf(0x81.toByte(), 126, (payload.size shr 8).toByte(), payload.size.toByte())
            }
            synchronized(out) {
                out.write(header)
                out.write(payload)
                out.flush()
            }
        }
    }

    @Before
    fun setUp() {
        context = InstrumentationRegistry.getInstrumentation().targetContext
        notifications = context.getSystemService(NotificationManager::class.java)
        notifications.cancelAll()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            InstrumentationRegistry.getInstrumentation().uiAutomation
                .grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
        }
        server = ServerSocket(0, 8, InetAddress.getByName("127.0.0.1"))
        origin = "http://127.0.0.1:${server.localPort}"
        Thread { serve() }.apply { isDaemon = true }.start()

        val generator = KeyPairGenerator.getInstance("EC")
        generator.initialize(ECGenParameterSpec("secp256r1"))
        val phone = generator.generateKeyPair()
        phonePublic = point(phone.public as ECPublicKey)
        auth = ByteArray(16).also { SecureRandom().nextBytes(it) }
        val privateKey = JSONObject()
            .put("kty", "EC")
            .put("crv", "P-256")
            .put("d", b64(unsigned((phone.private as ECPrivateKey).s)))
        val account = JSONObject()
            .put("subscription", SUBSCRIPTION)
            .put("origin", origin)
            .put("userId", USER)
            .put("refreshToken", "refresh")
            .put("sessionToken", "session")
            .put("keys", JSONObject().put("privateKey", privateKey).put("publicKey", b64(phonePublic)).put("auth", b64(auth)))
            .put("applicationServerKey", "key")
            .put("deploymentSubscription", "dep")
        val state = JSONObject()
            .put("version", 1)
            .put("device", JSONObject.NULL)
            .put("accounts", JSONArray().put(account))
        PushStore(context).save(state.toString())
    }

    @After
    fun tearDown() {
        server.close()
        notifications.cancelAll()
    }

    @Test
    fun ringsFromThePushAndStopsWhenTheRingEndsElsewhere() {
        val channel = "0190f0a0-0000-7000-8000-000000000011"
        val session = "0190f0a0-0000-7000-8000-000000000022"
        val pointer = JSONObject()
            .put("v", 1)
            .put("kind", "call")
            .put("channel", channel)
            .put("session", session)
            .put("caller", CALLER)
            .put("callerName", "Alice")
            .put("until", Instant.now().plusSeconds(30).toString())
        val handled = System.nanoTime()
        PushHandler(context).handle(SUBSCRIPTION, encrypt(pointer))

        val shown = waitFor("the incoming call's notification", 5_000) { incomingNotification() }
        val afterMillis = (System.nanoTime() - handled) / 1_000_000
        assertNotNull("posted within five seconds (took $afterMillis ms)", shown)
        val extras = shown!!.extras
        assertEquals("Alice", extras.getCharSequence(Notification.EXTRA_TITLE).toString())
        assertEquals(Notification.CATEGORY_CALL, shown.category)
        assertTrue("the notification can wake the lock screen", shown.fullScreenIntent != null)
        val engine = waitFor("the deployment's engine", 5_000) { CallEngines.existing(origin) }!!
        val telecom = engine.telecom
        assertTrue(
            "Telecom holds the call as ringing (${telecom.phase})",
            waitFor("the ringing phase", 5_000) { if (telecom.phase == CallTelecom.Phase.RINGING) true else null } == true,
        )
        val control = waitFor("Telecom's id for the call", 5_000) { telecom.callId }
        assertNotNull("Telecom added the call", control)

        // The ring watch identifies on the deployment's event stream with the account's session.
        val stream = identified.poll(10, TimeUnit.SECONDS)
        assertNotNull("the ring watch opened the event stream", stream)
        assertEquals("session", stream!!.identify.getString("sessionToken"))
        stream.send(
            JSONObject()
                .put("type", "ready")
                .put("userId", USER)
                .put("resumed", false)
                .put("replayComplete", true)
                .put("catchUpFollows", true)
                .put("serverTime", Instant.now().toString()),
        )
        // Someone else's ring ending changes nothing; the user's own ends the call.
        stream.send(event(1, JSONObject().put("serverEvent", "voiceRing").put("type", "delete").put("session", session).put("user", CALLER)))
        Thread.sleep(500)
        assertEquals(CallTelecom.Phase.RINGING, telecom.phase)
        assertNotNull(incomingNotification())
        stream.send(event(2, JSONObject().put("serverEvent", "voiceRing").put("type", "delete").put("session", session).put("user", USER)))

        assertTrue(
            "the ring's end elsewhere ends the call (${telecom.phase})",
            waitFor("the call to end", 10_000) { if (telecom.phase == CallTelecom.Phase.IDLE) true else null } == true,
        )
        assertNull(
            "and takes the notification down",
            waitFor("the notification to go", 5_000) { if (incomingNotification() == null) null else Unit },
        )
        assertEquals(CallTelecom.Phase.IDLE, telecom.phase)
    }

    @Test
    fun aPushThatCannotBeReadRingsNothing() {
        val garbage = Base64.getUrlDecoder().decode(
            encrypt(JSONObject().put("v", 1).put("kind", "call").put("channel", "c").put("session", "s").put("until", "x")),
        )
        garbage[garbage.size - 1] = (garbage[garbage.size - 1].toInt() xor 1).toByte()
        PushHandler(context).handle(SUBSCRIPTION, b64(garbage))
        PushHandler(context).handle("someone-else", encrypt(JSONObject().put("v", 1).put("kind", "call")))
        Thread.sleep(1_000)
        assertNull(incomingNotification())
        assertTrue(identified.isEmpty())
        assertNull(CallEngines.existing(origin)?.telecom?.phase?.takeIf { it != CallTelecom.Phase.IDLE })
    }

    private fun incomingNotification(): Notification? =
        notifications.activeNotifications.firstOrNull { it.id == CallService.NOTIFICATION_ID }?.notification

    /** What `check` first answers within `millis`, polling; `null` when it never did. */
    private fun <T> waitFor(what: String, millis: Long, check: () -> T?): T? {
        val deadline = System.currentTimeMillis() + millis
        while (System.currentTimeMillis() < deadline) {
            check()?.let { return it }
            Thread.sleep(50)
        }
        return check()
    }

    private fun event(sequence: Int, event: JSONObject): JSONObject =
        JSONObject().put("type", "event").put("sequence", sequence).put("event", event)

    /** The pointer, encrypted to the phone as a deployment does (RFC 8291), base64url. */
    private fun encrypt(pointer: JSONObject): String {
        val generator = KeyPairGenerator.getInstance("EC")
        generator.initialize(ECGenParameterSpec("secp256r1"))
        val sender = generator.generateKeyPair()
        val senderPublic = point(sender.public as ECPublicKey)
        val agreement = KeyAgreement.getInstance("ECDH")
        agreement.init(sender.private)
        agreement.doPhase(
            KeyFactory.getInstance("EC").generatePublic(ECPublicKeySpec(WebPush.point(phonePublic), WebPush.p256())),
            true,
        )
        val secret = agreement.generateSecret()
        val salt = ByteArray(16).also { SecureRandom().nextBytes(it) }
        val ikm = WebPush.hkdf(auth, secret, "WebPush: info\u0000".toByteArray(StandardCharsets.US_ASCII) + phonePublic + senderPublic, 32)
        val cek = WebPush.hkdf(salt, ikm, "Content-Encoding: aes128gcm\u0000".toByteArray(StandardCharsets.US_ASCII), 16)
        val nonce = WebPush.hkdf(salt, ikm, "Content-Encoding: nonce\u0000".toByteArray(StandardCharsets.US_ASCII), 12)
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, SecretKeySpec(cek, "AES"), GCMParameterSpec(128, nonce))
        val record = cipher.doFinal(pointer.toString().toByteArray(StandardCharsets.UTF_8) + byteArrayOf(2))
        return b64(salt + byteArrayOf(0, 0, 16, 0, 65) + senderPublic + record)
    }

    /**
     * The stand-in deployment: its event stream alone, as a WebSocket server of the standard
     * library's sockets, keeping each connection once it has identified.
     */
    private fun serve() {
        while (!server.isClosed) {
            val socket = try {
                server.accept()
            } catch (e: Exception) {
                return
            }
            Thread { speak(socket) }.apply { isDaemon = true }.start()
        }
    }

    private fun speak(socket: Socket) {
        try {
            val input = socket.getInputStream()
            val reader = BufferedReader(InputStreamReader(input, StandardCharsets.ISO_8859_1))
            val request = reader.readLine() ?: return
            var key: String? = null
            while (true) {
                val header = reader.readLine() ?: return
                if (header.isEmpty()) break
                val (name, value) = header.split(":", limit = 2).let { it[0].trim().lowercase() to it.getOrElse(1) { "" }.trim() }
                if (name == "sec-websocket-key") key = value
            }
            val out = socket.getOutputStream()
            if (!request.startsWith("GET /api/v1/events") || key == null) {
                out.write("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".toByteArray())
                out.flush()
                socket.close()
                return
            }
            val accept = Base64.getEncoder().encodeToString(
                MessageDigest.getInstance("SHA-1").digest("$key${"258EAFA5-E914-47DA-95CA-C5AB0DC85B11"}".toByteArray(StandardCharsets.US_ASCII)),
            )
            out.write(
                ("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
                    "Sec-WebSocket-Accept: $accept\r\n\r\n").toByteArray(StandardCharsets.US_ASCII),
            )
            out.flush()
            // The buffered reader may have read ahead into the first frame; the client sends it
            // only once the upgrade is answered, so it has not.
            val identify = readFrame(input) ?: return
            identified.add(Stream(JSONObject(identify), out))
            // Frames after it (pings, nothing else) are read and dropped until the socket ends.
            while (readFrame(input) != null) {
                // Nothing to answer.
            }
        } catch (e: Exception) {
            // The connection ended; the test judges by what it was sent.
        }
    }

    /** One masked text frame from the client, unmasked; `null` once the socket is closed. */
    private fun readFrame(input: InputStream): String? {
        val first = input.read()
        val second = input.read()
        if (first < 0 || second < 0) return null
        var length = (second and 0x7f).toLong()
        if (length == 126L) {
            length = ((input.read() shl 8) or input.read()).toLong()
        } else if (length == 127L) {
            length = 0
            repeat(8) { length = (length shl 8) or input.read().toLong() }
        }
        val mask = ByteArray(4)
        if (second and 0x80 != 0) {
            readFully(input, mask)
        }
        val payload = ByteArray(length.toInt())
        readFully(input, payload)
        for (i in payload.indices) {
            payload[i] = (payload[i].toInt() xor mask[i % 4].toInt()).toByte()
        }
        return when (first and 0x0f) {
            0x8 -> null
            0x1 -> String(payload, StandardCharsets.UTF_8)
            // A ping or a binary frame; the next text frame is what is wanted.
            else -> readFrame(input)
        }
    }

    private fun readFully(input: InputStream, into: ByteArray) {
        var read = 0
        while (read < into.size) {
            val got = input.read(into, read, into.size - read)
            if (got < 0) throw IllegalStateException("the socket ended mid-frame")
            read += got
        }
    }

    private companion object {
        const val SUBSCRIPTION = "0190f0a0-0000-7000-8000-00000000bbbb"
        const val USER = "0190f0a0-0000-7000-8000-000000000001"
        const val CALLER = "0190f0a0-0000-7000-8000-000000000002"

        fun point(key: ECPublicKey): ByteArray = byteArrayOf(4) + unsigned(key.w.affineX) + unsigned(key.w.affineY)

        /** A coordinate or scalar as exactly 32 bytes. */
        fun unsigned(value: BigInteger): ByteArray {
            val raw = value.toByteArray()
            val out = ByteArray(32)
            val copy = minOf(raw.size, 32)
            System.arraycopy(raw, raw.size - copy, out, 32 - copy, copy)
            return out
        }

        fun b64(bytes: ByteArray): String = Base64.getUrlEncoder().withoutPadding().encodeToString(bytes)
    }
}
