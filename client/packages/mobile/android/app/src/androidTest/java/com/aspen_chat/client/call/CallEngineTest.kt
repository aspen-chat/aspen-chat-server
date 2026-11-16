package com.aspen_chat.client.call

import android.Manifest
import android.app.ActivityManager
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.media.AudioManager
import android.os.ParcelFileDescriptor
import android.os.ParcelUuid
import android.service.notification.StatusBarNotification
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.aspen_chat.client.push.PushStore
import com.aspen_chat.client.voice.CallState
import com.aspen_chat.client.voice.CallStatus
import com.aspen_chat.client.voice.Configuration
import com.aspen_chat.client.voice.ConnectedSocket
import com.aspen_chat.client.voice.ConnectionState
import com.aspen_chat.client.voice.CreatedPeerConnection
import com.aspen_chat.client.voice.MediaSource
import com.aspen_chat.client.voice.PeerConnection
import com.aspen_chat.client.voice.PeerConnectionFactory
import com.aspen_chat.client.voice.PeerConnectionObserver
import com.aspen_chat.client.voice.Socket
import com.aspen_chat.client.voice.SocketFactory
import com.aspen_chat.client.voice.StatsOf
import java.net.HttpURLConnection
import java.net.URL
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The call engine with this device's whole platform under it, against a real deployment: it joins
 * a voice channel's call, which Telecom holds as the app's own call (dialing, then active) with the
 * call's service and notification up and its routes listed and switched, is heard by and hears a
 * second participant in a browser, takes its socket back after the socket is cut, and leaves,
 * which ends the system's side too. `scripts/check_mobile_call.py --platform android`
 * at the repository's root runs the deployment, the browser, and this test, and gives it what it
 * reads from the instrumentation's arguments (`-Pandroid.testInstrumentationRunnerArguments.<name>`);
 * without them it is skipped.
 *
 * | Argument                   | What                                                        |
 * | -------------------------- | ----------------------------------------------------------- |
 * | `ASPEN_CALL_API`           | The deployment's `public_url`, as the device reaches it     |
 * | `ASPEN_CALL_CHANNEL`       | The voice channel to join                                   |
 * | `ASPEN_CALL_USER_ID`       | The user whose call this is                                 |
 * | `ASPEN_CALL_REFRESH_TOKEN` | Their refresh token; the session token kept is a stale one, |
 * |                            | so the first request is refused and renewed from this       |
 * | `ASPEN_CALL_HELPER`        | The script's helper, which brings the browser in            |
 */
@RunWith(AndroidJUnit4::class)
class CallEngineTest {
    /** `CallSockets` with every socket opened kept, so the test can cut the live one. */
    class CuttableSockets : SocketFactory {
        private val real = CallSockets()
        private val opened = ArrayList<Socket>()

        override suspend fun connect(url: String, bearer: String?): ConnectedSocket {
            val connected = real.connect(url, bearer)
            synchronized(opened) { opened.add(connected.socket) }
            return connected
        }

        val connections: Int
            get() = synchronized(opened) { opened.size }

        /** Closes the socket opened last from this side, with no `leave` sent on it, as a network that drops it would. */
        fun cut() {
            synchronized(opened) { opened.lastOrNull() }?.closeSocket()
        }
    }

    /** Peer connections from `real`, with every one made kept for its statistics. */
    class RecordingPeerConnections(private val real: CallPeerConnections) : PeerConnectionFactory {
        private val created = ArrayList<PeerConnection>()

        override suspend fun create(configuration: Configuration, observer: PeerConnectionObserver): CreatedPeerConnection {
            val made = real.create(configuration, observer)
            synchronized(created) { created.add(made.connection) }
            return made
        }

        val count: Int
            get() = synchronized(created) { created.size }

        val latest: PeerConnection?
            get() = synchronized(created) { created.lastOrNull() }
    }

    /** What an `inbound-rtp` audio entry of a statistics report measured. */
    data class InboundAudio(val packets: Long, val energy: Double, val level: Double)

    /** What the microphone's `media-source` entry measured: `totalAudioEnergy` and `audioLevel`. */
    data class SourceAudio(val energy: Double, val level: Double)

    /** Snapshots as the listener saw them, on the main thread. */
    class Seen : CallSnapshots.Listener {
        private val snapshots = ArrayList<CallState>()

        override fun changed(state: CallState) {
            synchronized(snapshots) { snapshots.add(state) }
        }

        fun clear() = synchronized(snapshots) { snapshots.clear() }

        val statuses: List<CallStatus>
            get() = synchronized(snapshots) { snapshots.map { it.status } }

        val revisions: List<ULong>
            get() = synchronized(snapshots) { snapshots.map { it.revision } }
    }

    private lateinit var context: Context
    private lateinit var store: PushStore
    private var keptPushState: String? = null
    private var helper: String? = null

    private fun argument(name: String): String {
        val value = InstrumentationRegistry.getArguments().getString(name)
        assumeTrue("$name is not given", !value.isNullOrEmpty())
        return value!!
    }

    @Before
    fun setUp() {
        context = InstrumentationRegistry.getInstrumentation().targetContext
        store = PushStore(context)
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        automation.grantRuntimePermission(context.packageName, Manifest.permission.RECORD_AUDIO)
        // The call's notification, which the app asks leave for when it signs up for pushes.
        automation.grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
    }

    @After
    fun tearDown() {
        val kept = keptPushState
        if (kept != null) store.save(kept) else store.remove()
        helper?.let { http("POST", "$it/second-participant/leave") }
    }

    @Test
    fun joinsHearsASecondParticipantResumesAndLeaves() = runBlocking {
        val api = argument("ASPEN_CALL_API")
        val channel = argument("ASPEN_CALL_CHANNEL")
        val user = argument("ASPEN_CALL_USER_ID")
        val refreshToken = argument("ASPEN_CALL_REFRESH_TOKEN")
        val helper = argument("ASPEN_CALL_HELPER")
        this@CallEngineTest.helper = helper

        // The account as the web view keeps it on sign-in, with a session token the deployment
        // refuses, so the first request goes through the renewal.
        keptPushState = store.load()
        val account = JSONObject()
            .put("subscription", "call-engine-test")
            .put("origin", api)
            .put("userId", user)
            .put("refreshToken", refreshToken)
            .put("sessionToken", "stale-session-token")
            .put("keys", JSONObject().put("privateKey", JSONObject().put("d", "AA")).put("publicKey", "AA").put("auth", "AA"))
        store.save(JSONObject().put("version", 1).put("accounts", JSONArray().put(account)).toString())

        // WebRTC on the real audio device module: the emulator's microphone is whatever the
        // machine running it gives, which the check feeds a tone.
        val media = CallEngine.peerConnections(context)
        val sockets = CuttableSockets()
        val peerConnections = RecordingPeerConnections(media)
        val call = CallEngine(context, api, loopbackPlainText = true, media = media, sockets = sockets, peerConnections = peerConnections)
        val seen = Seen()
        val listening = call.snapshots.listen(seen)
        val audioManager = context.getSystemService(AudioManager::class.java)
        val modeBefore = audioManager.mode
        try {
            val microphone = call.audio.openMicrophone()
            call.engine.setMicrophone(microphone)
            assertEquals(CallTelecom.Phase.IDLE, call.telecom.phase)
            assertTrue("no routes outside a call", call.routes.routes.isEmpty())
            call.engine.join(channel)

            // The system's call follows the engine's: outgoing while the engine joins, and
            // active once it connects, with the service and its notification up meanwhile.
            val dialing = untilTelecom("the system call is dialing", 20, call) { it == CallTelecom.Phase.DIALING }
            assertEquals(CallTelecom.Phase.DIALING, dialing)

            // Joined: the microphone is sent and the renewed session token is kept.
            val connected = until("the call connects", 40, call) { it.status == CallStatus.CONNECTED }
            untilTelecom("the system call is active", 20, call) { it == CallTelecom.Phase.ACTIVE }
            val callId = call.telecom.callId ?: throw AssertionError("Telecom gave the call no id")
            val telecomActive = telecomLines(callId)
            assertTrue("Telecom lists the call $callId:\n$telecomActive", telecomActive.isNotEmpty())
            assertEquals("the call plays in the communication mode", AudioManager.MODE_IN_COMMUNICATION, audioManager.mode)
            val service = awaitValue("the call's service runs in the foreground", 10) { callService()?.takeIf { it.foreground } }
            val notice = awaitValue("the call's notification is posted", 10) { callNotification() }
            assertEquals(Notification.CATEGORY_CALL, notice.notification.category)
            assertTrue("the notification is the service's", notice.notification.flags and Notification.FLAG_FOREGROUND_SERVICE != 0)
            assertTrue("the notification stays", notice.notification.flags and Notification.FLAG_ONGOING_EVENT != 0)

            // The routes are Telecom's endpoints, typed; the loudspeaker is among them on any
            // phone, and asking for it makes it the current one.
            val routes = awaitValue("Telecom lists the call's routes", 10) { call.routes.routes.takeIf { it.isNotEmpty() } }
            assertTrue("every route has a kind: $routes", routes.all { it.kind != CallRoutes.Kind.OTHER })
            assertEquals("one route is current: $routes", 1, routes.count { it.current })
            assertTrue("the loudspeaker is a route: $routes", routes.any { it.kind == CallRoutes.Kind.SPEAKER })
            assertTrue(call.routes.select(CallRoutes.Kind.SPEAKER))
            awaitTrue("the call moves to the loudspeaker", 10) { call.routes.currentKind == CallRoutes.Kind.SPEAKER }
            val speaking = call.routes.routes
            assertTrue("the loudspeaker is current: $speaking", speaking.single { it.current }.kind == CallRoutes.Kind.SPEAKER)
            val earpiece = routes.firstOrNull { it.kind == CallRoutes.Kind.EARPIECE }
            if (earpiece != null) {
                assertTrue(call.routes.hasEarpiece)
                assertTrue(call.routes.select(CallRoutes.Kind.EARPIECE))
                awaitTrue("the call moves back to the earpiece", 10) { call.routes.currentKind == CallRoutes.Kind.EARPIECE }
            }
            assertTrue("no route of a kind the phone lacks", !call.routes.select(CallRoutes.Kind.WIRED_HEADSET) || routes.any { it.kind == CallRoutes.Kind.WIRED_HEADSET })
            println(
                "call-engine-test: Telecom holds the call $callId (${telecomActive.first().trim()}); " +
                    "${service.service.shortClassName} runs in the foreground with its notification; " +
                    "routes ${routes.map { "${it.kind}(${it.name})" }}, earpiece ${call.routes.hasEarpiece}, " +
                    "moved to ${speaking.single { it.current }.kind}${if (earpiece != null) " and back to EARPIECE" else ""}",
            )
            assertEquals(channel, connected.channel)
            assertEquals(user, connected.user)
            assertNotNull(connected.session)
            assertTrue(connected.grants.speak)
            assertEquals(microphone, connected.microphone)
            assertNotEquals("the renewed token is kept", "stale-session-token", call.credentials.sessionToken())
            val sending = until("the microphone is live at the server", 15, call) {
                it.sources.any { source -> source.source == MediaSource.MICROPHONE && source.live }
            }
            assertEquals(listOf(microphone), sending.sources.map { it.track })
            assertEquals(1, sockets.connections)
            assertEquals(1, peerConnections.count)
            val connection = peerConnections.latest ?: throw AssertionError("no peer connection")
            assertEquals(ConnectionState.CONNECTED, connection.connectionState())

            // A second participant from a browser, playing a tone: they take an audio slot, whose
            // section the connection has a track for, and their sound arrives on it.
            val second = JSONObject(http("POST", "$helper/second-participant"))
            val bob = second.getString("user")
            val heard = until("the second participant takes a slot", 30, call) { state ->
                state.participants.any { it.user == bob } && state.slots.any { it.user == bob && it.track != null }
            }
            assertTrue(heard.audioSlots.toInt() > 0)
            assertEquals("every slot has a track", heard.slots.size, heard.slots.count { it.track != null })
            val before = inboundAudio(connection)
            delay(4_000)
            val after = inboundAudio(connection)
            assertTrue("packets arrive: $before then $after", after.packets > before.packets + 100)
            assertTrue("the tone's energy rises: $before then $after", after.energy > before.energy)
            assertTrue("the tone has a level: $after", after.level > 0)

            // And the browser hears this device's microphone: packets arrive and decode, and the
            // sound off its track is what the microphone carried. The device's own source
            // statistics say whether that was sound (a tone played to it) or silence (an
            // emulator whose host audio never reached it); the browser must hear the same.
            val browserBefore = browserInbound(helper)
            delay(4_000)
            val browserAfter = browserInbound(helper)
            val source = sourceAudio(connection)
            assertTrue("the browser receives audio: $browserBefore then $browserAfter", browserAfter.first > browserBefore.first)
            assertTrue("and decodes it: $browserBefore then $browserAfter", browserAfter.second > browserBefore.second)
            val browserPeak = JSONObject(http("GET", "$helper/second-participant/loudness")).getDouble("peak")
            val microphoneSounding = source.energy > 0.001
            if (microphoneSounding) {
                assertTrue("and hears the microphone's sound in it: $browserPeak (this side's source: $source)", browserPeak > 0.01)
            } else {
                assertTrue("and hears the microphone's silence in it: $browserPeak (this side's source: $source)", browserPeak < 0.01)
            }

            // The socket cut: the engine takes its place back with `resume` on a new socket, media
            // flowing meanwhile on the same peer connection.
            val session = connected.session
            seen.clear()
            sockets.cut()
            val resumed = until("the call resumes", 20, call) { state ->
                state.status == CallStatus.CONNECTED && seen.statuses.contains(CallStatus.REJOINING)
            }
            assertEquals("the same session", session, resumed.session)
            assertEquals("one socket more", 2, sockets.connections)
            assertEquals("the same peer connection", 1, peerConnections.count)
            assertTrue("the slots stand", resumed.slots.any { it.user == bob })
            val afterResume = inboundAudio(connection)
            delay(2_000)
            val later = inboundAudio(connection)
            assertTrue("media flows on: $afterResume then $later", later.packets > afterResume.packets)

            // Leaving ends everything: the state is idle and the connection closed.
            call.engine.leave()
            val left = until("the call is left", 10, call) { it.status == CallStatus.IDLE }
            assertNull(left.session)
            assertTrue(left.slots.isEmpty())
            assertEquals(ConnectionState.CLOSED, connection.connectionState())
            assertEquals("the observer saw the latest snapshot", call.engine.state(), call.snapshots.latest)
            val revisions = seen.revisions
            assertTrue(
                "revisions count up by one: $revisions",
                revisions.zipWithNext().all { (before, after) -> after == before + 1uL },
            )
            // And the system's side with it: Telecom's call is gone, the service and its
            // notification are down, the routes empty, and the audio mode given back.
            untilTelecom("the system call ends", 20, call) { it == CallTelecom.Phase.IDLE }
            awaitTrue("the call's service stops", 10) { callService() == null }
            awaitTrue("the call's notification is taken down", 10) { callNotification() == null }
            awaitTrue("the audio mode is given back", 10) { audioManager.mode == modeBefore }
            assertTrue("no routes after the call", call.routes.routes.isEmpty())
            assertNull(call.routes.currentKind)
            val telecomAfter = telecomLines(callId)
            assertTrue("Telecom no longer holds the call $callId:\n$telecomAfter", telecomAfter.none { it.contains("ACTIVE") || it.contains("DIALING") })
            println("call-engine-test: left; Telecom's call ended, the service and its notification are down, routes empty, audio mode $modeBefore again")
            println(
                "call-engine-test: joined, heard ${after.packets - before.packets} packets of the browser's tone in 4 s " +
                    "(energy ${before.energy} → ${after.energy}, level ${after.level}); this side's microphone source " +
                    "${if (microphoneSounding) "sounded" else "was silent"} (energy ${source.energy}, level ${source.level}); " +
                    "the browser received ${browserAfter.first - browserBefore.first} packets and " +
                    "${browserAfter.second - browserBefore.second} samples in 4 s, peaking at $browserPeak; " +
                    "resumed on socket 2 with session $session; left",
            )
        } finally {
            listening.cancel()
            call.audio.closeMicrophone()
            call.close()
        }
    }

    // --- Helpers --------------------------------------------------------------------------------

    /** The engine's state once `holds` of it, or a failure after `seconds`, naming the last state. */
    private suspend fun until(what: String, seconds: Long, call: CallEngine, holds: (CallState) -> Boolean): CallState {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(seconds)
        var state = call.engine.state()
        while (!holds(state)) {
            if (System.nanoTime() > deadline) {
                fail("$what did not happen within $seconds s: $state")
            }
            delay(100)
            state = call.engine.state()
        }
        return state
    }

    /** The system call's phase once `holds` of it, or a failure after `seconds`. */
    private suspend fun untilTelecom(what: String, seconds: Long, call: CallEngine, holds: (CallTelecom.Phase) -> Boolean): CallTelecom.Phase {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(seconds)
        var phase = call.telecom.phase
        while (!holds(phase)) {
            if (System.nanoTime() > deadline) {
                fail("$what did not happen within $seconds s: $phase")
            }
            delay(100)
            phase = call.telecom.phase
        }
        return phase
    }

    /** What `read` answers once it is not `null`, or a failure after `seconds`. */
    private suspend fun <T : Any> awaitValue(what: String, seconds: Long, read: () -> T?): T {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(seconds)
        while (true) {
            read()?.let { return it }
            if (System.nanoTime() > deadline) {
                fail("$what did not happen within $seconds s")
            }
            delay(100)
        }
    }

    /** Waits until `holds`, or fails after `seconds`. */
    private suspend fun awaitTrue(what: String, seconds: Long, holds: () -> Boolean) {
        awaitValue(what, seconds) { if (holds()) Unit else null }
    }

    /** The call's service among the app's running services, with whether it is in the foreground. */
    @Suppress("DEPRECATION")
    private fun callService(): ActivityManager.RunningServiceInfo? =
        context.getSystemService(ActivityManager::class.java).getRunningServices(100)
            .firstOrNull { it.service.className == CallService::class.java.name }

    private fun callNotification(): StatusBarNotification? =
        context.getSystemService(NotificationManager::class.java).activeNotifications.firstOrNull { it.id == CallService.NOTIFICATION_ID }

    /** The lines of `dumpsys telecom` that name the call `id`, Telecom's own word on it. */
    private fun telecomLines(id: ParcelUuid): List<String> {
        val descriptor = InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand("dumpsys telecom")
        val text = ParcelFileDescriptor.AutoCloseInputStream(descriptor).bufferedReader().use { it.readText() }
        return text.lines().filter { it.contains(id.toString()) }
    }

    /** The body of `url`'s answer, which must be `200`. */
    private fun http(method: String, url: String): String {
        val connection = URL(url).openConnection() as HttpURLConnection
        connection.requestMethod = method
        connection.connectTimeout = 60_000
        connection.readTimeout = 60_000
        val status = connection.responseCode
        val body = (if (status < 400) connection.inputStream else connection.errorStream)?.bufferedReader()?.readText() ?: ""
        assertEquals("$method $url: $body", 200, status)
        return body
    }

    private suspend fun entries(connection: PeerConnection): List<JSONObject> {
        val array = JSONArray(connection.stats(StatsOf.Connection))
        return (0 until array.length()).map { array.getJSONObject(it) }
    }

    /** The connection's inbound audio, summed over its `inbound-rtp` audio entries. */
    private suspend fun inboundAudio(connection: PeerConnection): InboundAudio {
        var summed = InboundAudio(0, 0.0, 0.0)
        for (entry in entries(connection)) {
            if (entry.optString("type") == "inbound-rtp" && entry.optString("kind") == "audio") {
                summed = InboundAudio(
                    packets = summed.packets + entry.optLong("packetsReceived"),
                    energy = summed.energy + entry.optDouble("totalAudioEnergy", 0.0),
                    level = maxOf(summed.level, entry.optDouble("audioLevel", 0.0)),
                )
            }
        }
        return summed
    }

    /** What the microphone's source has captured (`media-source`), after WebRTC's processing. */
    private suspend fun sourceAudio(connection: PeerConnection): SourceAudio {
        val source = entries(connection).firstOrNull { it.optString("type") == "media-source" && it.optString("kind") == "audio" }
        return SourceAudio(source?.optDouble("totalAudioEnergy", 0.0) ?: 0.0, source?.optDouble("audioLevel", 0.0) ?: 0.0)
    }

    /** The packets and samples the browser's connection received of audio, from the helper's report of it. */
    private fun browserInbound(helper: String): Pair<Long, Long> {
        val streams = JSONObject(http("GET", "$helper/second-participant/report")).getJSONArray("streams")
        var packets = 0L
        var samples = 0L
        for (i in 0 until streams.length()) {
            val stream = streams.getJSONObject(i)
            if (stream.optString("direction") == "inbound" && stream.optString("kind") == "audio") {
                packets += stream.optLong("packets")
                samples += stream.optLong("samples")
            }
        }
        return packets to samples
    }
}
