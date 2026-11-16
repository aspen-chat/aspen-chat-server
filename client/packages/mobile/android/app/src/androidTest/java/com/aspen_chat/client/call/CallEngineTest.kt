package com.aspen_chat.client.call

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.media.AudioManager
import android.media.MediaPlayer
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.aspen_chat.client.push.PushStore
import com.aspen_chat.client.call.CallTestSupport.argument
import com.aspen_chat.client.call.CallTestSupport.awaitTrue
import com.aspen_chat.client.call.CallTestSupport.awaitValue
import com.aspen_chat.client.call.CallTestSupport.browserInbound
import com.aspen_chat.client.call.CallTestSupport.browserOnHold
import com.aspen_chat.client.call.CallTestSupport.browserVideoFrames
import com.aspen_chat.client.call.CallTestSupport.callNotification
import com.aspen_chat.client.call.CallTestSupport.callService
import com.aspen_chat.client.call.CallTestSupport.http
import com.aspen_chat.client.call.CallTestSupport.inboundAudio
import com.aspen_chat.client.call.CallTestSupport.shell
import com.aspen_chat.client.call.CallTestSupport.sourceAudio
import com.aspen_chat.client.call.CallTestSupport.telecomLines
import com.aspen_chat.client.call.CallTestSupport.until
import com.aspen_chat.client.call.CallTestSupport.untilTelecom
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
import com.aspen_chat.client.voice.SoundCue
import com.aspen_chat.client.voice.TrackId
import com.aspen_chat.client.voice.WatchedVideo
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.coroutines.suspendCoroutine
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.webrtc.VideoFrame
import org.webrtc.VideoSink
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The call engine with this device's whole platform under it, against a real deployment: it joins
 * a voice channel's call, which Telecom holds as the app's own call (dialing, then active) with the
 * call's service and notification up and its routes listed and switched, is heard by and hears a
 * second participant in a browser, holds the call and releases it, watches the browser's camera
 * (its track retained as `CallVideo` retains one, a sink counting its frames and reading their
 * shape), sends the emulator's own camera (`CallCamera`), which the browser decodes and sees a
 * picture in, moves it to the other camera, takes its socket back after the socket is cut with
 * video flowing on, and leaves, which ends the system's side too. `scripts/check_mobile_call.py --platform android`
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
 * | `ASPEN_CALL_MICROPHONE`    | What the browser is to hear of this device's microphone:    |
 * |                            | `silent` (an emulator's, whose host audio never reaches     |
 * |                            | it) or `live` (a phone's, which hears the room)             |
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

        /** What the socket opened last negotiated (`Sec-WebSocket-Extensions`), or "none". */
        val latestExtensions: String
            get() = synchronized(opened) { (opened.lastOrNull() as? CallSocket)?.negotiatedExtensions ?: "none" }

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

    /** What this device's microphone carries, as the check running the test says (`ASPEN_CALL_MICROPHONE`). */
    enum class Microphone { SILENT, LIVE }

    /** A frame of someone's camera as the browser plays it (the page's `cameraSample`). */
    data class CameraSample(val width: Int, val height: Int, val brightness: Double, val spread: Double)

    /** A sink counting the frames a video track delivers and keeping the latest one's upright shape. */
    class CountingSink : VideoSink {
        @Volatile
        var frames = 0
            private set
        @Volatile
        var shape: Triple<Int, Int, Int>? = null
            private set

        override fun onFrame(frame: VideoFrame) {
            frames += 1
            shape = Triple(frame.rotatedWidth, frame.rotatedHeight, frame.rotation)
        }
    }

    /**
     * Each of the call's sounds as the layer started playing it, on the main thread: the cue,
     * whether its player was playing as it was given, and, for the first joining, the lines of
     * `dumpsys audio` naming this process's players at that moment, the system's own word on
     * what plays and as what.
     */
    class SoundsSeen : CallSounds.Listener {
        data class Played(val cue: SoundCue, val playing: Boolean, val players: List<String>)

        private val recorded = ArrayList<Played>()

        override fun played(cue: SoundCue, player: MediaPlayer) {
            val first = synchronized(recorded) { recorded.none { it.cue == SoundCue.CALL_JOINED } }
            val players = if (cue == SoundCue.CALL_JOINED && first) audioPlayerLines() else emptyList()
            synchronized(recorded) { recorded.add(Played(cue, player.isPlaying, players)) }
        }

        val played: List<Played>
            get() = synchronized(recorded) { recorded.toList() }

        /** The lines of `dumpsys audio` that name a player of this process. */
        private fun audioPlayerLines(): List<String> {
            val pid = "/${android.os.Process.myPid()}"
            return shell("dumpsys audio").lines().filter { it.contains(pid) && it.contains("AudioAttributes") }
        }
    }

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

    @Before
    fun setUp() {
        context = InstrumentationRegistry.getInstrumentation().targetContext
        store = PushStore(context)
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        automation.grantRuntimePermission(context.packageName, Manifest.permission.RECORD_AUDIO)
        automation.grantRuntimePermission(context.packageName, Manifest.permission.CAMERA)
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
        val expectedMicrophone = when (val given = argument("ASPEN_CALL_MICROPHONE")) {
            "silent" -> Microphone.SILENT
            "live" -> Microphone.LIVE
            else -> throw AssertionError("ASPEN_CALL_MICROPHONE is $given; it is silent or live")
        }
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
            // The call's sounds, as the engine cues them and the layer plays them: each is
            // recorded as it starts, with the player's state, and for the first the system's own
            // word on it (`dumpsys audio`'s players, read on the spot while it plays).
            val sounds = SoundsSeen()
            val soundsListening = call.sounds.listen(sounds)
            call.engine.join(channel)

            // The system's call follows the engine's: outgoing while the engine joins, and
            // active once it connects, with the service and its notification up meanwhile.
            val dialing = untilTelecom("the system call is dialing", 20, call) { it == CallTelecom.Phase.DIALING }
            assertEquals(CallTelecom.Phase.DIALING, dialing)

            // Joined: the microphone is sent and the renewed session token is kept.
            val connected = until("the call connects", 40, call) { it.status == CallStatus.CONNECTED }
            awaitTrue("the joining is played", 10) { sounds.played.any { it.cue == SoundCue.CALL_JOINED } }
            val joined = sounds.played.first { it.cue == SoundCue.CALL_JOINED }
            assertTrue("the player plays as the cue is given", joined.playing)
            assertEquals("nothing before the connection, and the joining once", listOf(SoundCue.CALL_JOINED), sounds.played.map { it.cue })
            // The sound's own player, told apart from WebRTC's voice track (an `AudioTrack` of
            // speech on the same usage) by its kind and content type.
            val voicePlayers = joined.players.filter {
                it.contains("android.media.MediaPlayer") && it.contains("USAGE_VOICE_COMMUNICATION") && it.contains("CONTENT_TYPE_SONIFICATION")
            }
            assertTrue("the system lists the sound as a MediaPlayer of this process playing a voice communication:\n${joined.players}", voicePlayers.isNotEmpty())
            assertTrue("and as started:\n$voicePlayers", voicePlayers.any { it.contains("state:started") })
            untilTelecom("the system call is active", 20, call) { it == CallTelecom.Phase.ACTIVE }
            val callId = call.telecom.callId ?: throw AssertionError("Telecom gave the call no id")
            val telecomActive = telecomLines(callId)
            assertTrue("Telecom lists the call $callId:\n$telecomActive", telecomActive.isNotEmpty())
            assertEquals("the call plays in the communication mode", AudioManager.MODE_IN_COMMUNICATION, audioManager.mode)
            val service = awaitValue("the call's service runs in the foreground", 10) { callService(context)?.takeIf { it.foreground } }
            val notice = awaitValue("the call's notification is posted", 10) { callNotification(context) }
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
            // Someone else joining is the joining sound again, played as the first was.
            awaitTrue("the browser's joining is played", 10) { sounds.played.count { it.cue == SoundCue.CALL_JOINED } == 2 }
            assertTrue(sounds.played.all { it.playing })
            val before = inboundAudio(connection)
            delay(4_000)
            val after = inboundAudio(connection)
            assertTrue("packets arrive: $before then $after", after.packets > before.packets + 100)
            assertTrue("the tone's energy rises: $before then $after", after.energy > before.energy)
            assertTrue("the tone has a level: $after", after.level > 0)

            // And the browser hears this device's microphone: packets arrive and decode, and the
            // sound off its track is what the microphone carried. What that is, the check says
            // (`ASPEN_CALL_MICROPHONE`): an emulator's microphone, whose host audio never reaches
            // it, carries silence, which the device's own source statistics confirm and the
            // browser must hear as silence; a phone's hears the room, so the browser reports the
            // level it hears, whatever it is, and nothing is asserted of how loud a room is.
            val browserBefore = browserInbound(helper)
            delay(4_000)
            val browserAfter = browserInbound(helper)
            val source = sourceAudio(connection)
            assertTrue("the browser receives audio: $browserBefore then $browserAfter", browserAfter.first > browserBefore.first)
            assertTrue("and decodes it: $browserBefore then $browserAfter", browserAfter.second > browserBefore.second)
            val browserPeak = JSONObject(http("GET", "$helper/second-participant/loudness")).getDouble("peak")
            assertTrue("the browser reports a level: $browserPeak", browserPeak.isFinite() && browserPeak >= 0.0)
            when (expectedMicrophone) {
                Microphone.SILENT -> {
                    assertTrue("this side's microphone is silent: $source", source.energy < 0.001)
                    assertTrue("and the browser hears its silence: $browserPeak (this side's source: $source)", browserPeak < 0.01)
                }
                Microphone.LIVE -> {
                    // The source's energy rises with what the room sounded like; the level alone
                    // is asserted, as reported. The signal came through either way: the browser
                    // decoded this side's packets above.
                    assertTrue("this side's source reports its level: $source", source.level.isFinite() && source.level >= 0.0)
                }
            }

            // On hold, as `CallTelecom` puts the call for Telecom's `onSetInactive` (which the
            // emulator, with no other call, never asks): shown at once, confirmed by the server,
            // which tells the browser, and the server sends neither side the other's sound
            // meanwhile. The release shows only on the server's word, and the sound comes back
            // with it.
            call.engine.setHeld(true)
            val held = until("the hold shows", 10, call) { it.held }
            assertTrue("the hold is beside the mute, not a mute", !held.muted)
            awaitTrue("the browser sees the phone on hold", 15) { browserOnHold(helper) == listOf(user) }
            delay(1_000)
            val heldPhone = inboundAudio(connection)
            val heldBrowser = browserInbound(helper)
            delay(2_000)
            val heldPhoneLater = inboundAudio(connection)
            val heldBrowserLater = browserInbound(helper)
            assertTrue("held, the phone is sent nothing: $heldPhone then $heldPhoneLater", heldPhoneLater.packets - heldPhone.packets < 20)
            assertTrue("held, the browser is sent nothing: $heldBrowser then $heldBrowserLater", heldBrowserLater.first - heldBrowser.first < 20)
            call.engine.setHeld(false)
            until("the server lifts the hold", 15, call) { !it.held }
            awaitTrue("the browser sees the phone off hold", 15) { browserOnHold(helper).isEmpty() }
            val released = inboundAudio(connection)
            delay(2_000)
            val releasedLater = inboundAudio(connection)
            assertTrue("released, the sound comes back: $released then $releasedLater", releasedLater.packets > released.packets + 50)
            val hold = "held (the browser listed $user on hold; ${heldPhoneLater.packets - heldPhone.packets} packets reached the phone " +
                "and ${heldBrowserLater.first - heldBrowser.first} the browser in 2 s) and released (${releasedLater.packets - released.packets} packets in 2 s)"

            // Video both ways. The browser shows video and turns its camera on (Chromium's fake
            // camera, a moving pattern); the engine is asked to watch it, gets a section for it,
            // and its track, retained as `CallVideo` retains one, delivers frames to a sink that
            // counts them and reads their shape, upright: the sender turns them, no orientation
            // extension being negotiated. (Drawing them needs a window; the bridge's proof has one.)
            http("POST", "$helper/second-participant/show-video")
            http("POST", "$helper/second-participant/camera/start")
            call.engine.setWatched(listOf(WatchedVideo(bob, MediaSource.CAMERA)))
            val watching = until("the browser's camera is received", 30, call) { state ->
                state.videos.any { it.user == bob && it.source == MediaSource.CAMERA }
            }
            val remoteSurface = watching.videos.first { it.user == bob && it.source == MediaSource.CAMERA }.track
            val remoteTrack = CallEngine.tracks.retainedVideoTrack(remoteSurface) ?: throw AssertionError("no video track $remoteSurface")
            val remoteFrames = CountingSink()
            remoteTrack.addSink(remoteFrames)
            awaitTrue("the browser's camera's frames arrive", 20) { remoteFrames.frames >= 5 }
            val remoteShape = remoteFrames.shape ?: throw AssertionError("no frame shape")
            assertTrue("a picture: $remoteShape", remoteShape.first > 0 && remoteShape.second > 0)
            assertEquals("received frames arrive upright", 0, remoteShape.third)

            // The emulator's camera, opened as the engine's camera source: the browser decodes the
            // frames and sees a picture in them rather than black; the preview's frames carry the
            // camera's rotation, which what is sent is turned by.
            val camera = CallEngine.camera(context)
            val cameras = camera.devices
            assertTrue("the emulator has cameras: $cameras", cameras.isNotEmpty())
            val opened = cameraStarted(camera, null, api)
            // The default is the front camera, or the first where the phone (this emulator) has none.
            assertEquals(cameras.firstOrNull { it.facing == CallCameraDevice.Facing.FRONT } ?: cameras.first(), opened.second)
            val selfFrames = CountingSink()
            camera.track!!.addSink(selfFrames)
            call.engine.startSource(MediaSource.CAMERA, opened.first)
            until("the camera is live at the server", 20, call) { state ->
                state.sources.any { it.source == MediaSource.CAMERA && it.live }
            }
            awaitTrue("the camera's frames arrive", 15) { selfFrames.frames >= 5 }
            val selfShape = selfFrames.shape ?: throw AssertionError("no camera frame shape")
            awaitTrue("the browser decodes the camera", 20) { browserVideoFrames(helper) > 10 }
            val sample = untilSample("the browser sees a picture in it", 20, helper, user)
            assertTrue("not black: $sample", sample.brightness > 20)
            assertTrue("a picture, not a point: $sample", sample.width > 0 && sample.height > 0)
            val other = cameras.firstOrNull { it.id != opened.second.id }
            val moved = if (other != null) {
                val moved = cameraStarted(camera, other.id, api)
                assertEquals(other, moved.second)
                assertEquals("the same track", opened.first, moved.first)
                "moved to ${other.id} (${other.facing})"
            } else {
                "no other camera to move to"
            }
            val video = "watched the browser's camera (${remoteFrames.frames} frames of ${remoteShape.first}×${remoteShape.second}); " +
                "sent the camera ${opened.second.id} (preview ${selfShape.first}×${selfShape.second} at ${selfShape.third}°), which the browser decoded " +
                "(${browserVideoFrames(helper)} frames, a frame of brightness ${sample.brightness.toInt()} and spread ${sample.spread.toInt()} at ${sample.width}×${sample.height}); $moved"

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
            val framesAfterResume = remoteFrames.frames
            delay(2_000)
            val later = inboundAudio(connection)
            assertTrue("media flows on: $afterResume then $later", later.packets > afterResume.packets)
            assertTrue("video flows on", remoteFrames.frames > framesAfterResume)
            assertTrue("the video watched stands", resumed.videos.any { it.track == remoteSurface })

            // The camera off: its source leaves the state and the capture stops.
            call.engine.stopSource(MediaSource.CAMERA)
            withContext(Dispatchers.Main) { camera.stop() }
            until("the camera is gone from the sources", 10, call) { state -> state.sources.none { it.source == MediaSource.CAMERA } }
            assertTrue(!camera.isOn)
            remoteTrack.removeSink(remoteFrames)
            CallEngine.tracks.releaseVideoTrack(remoteSurface, remoteTrack)

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
            awaitTrue("the call's service stops", 10) { callService(context) == null }
            awaitTrue("the call's notification is taken down", 10) { callNotification(context) == null }
            awaitTrue("the audio mode is given back", 10) { audioManager.mode == modeBefore }
            assertTrue("no routes after the call", call.routes.routes.isEmpty())
            assertNull(call.routes.currentKind)
            val telecomAfter = telecomLines(callId)
            assertTrue("Telecom no longer holds the call $callId:\n$telecomAfter", telecomAfter.none { it.contains("ACTIVE") || it.contains("DIALING") })
            // Leaving is the leaving sound; the cut socket was said lost and, with the resume,
            // back, as the web's call says of its own rejoining.
            awaitTrue("the leaving is played", 10) { sounds.played.any { it.cue == SoundCue.CALL_LEFT } }
            soundsListening.cancel()
            val played = sounds.played
            assertTrue("every sound's player played as it was given: $played", played.all { it.playing })
            assertEquals(
                listOf(SoundCue.CALL_JOINED, SoundCue.CALL_JOINED, SoundCue.DISCONNECTED, SoundCue.RECONNECTED, SoundCue.CALL_LEFT),
                played.map { it.cue },
            )
            println("call-engine-test: left; Telecom's call ended, the service and its notification are down, routes empty, audio mode $modeBefore again")
            println("call-engine-test: the sounds ${played.map { it.cue }} each played by a MediaPlayer as a voice communication (${voicePlayers.first().trim()})")
            println(
                "call-engine-test: joined, heard ${after.packets - before.packets} packets of the browser's tone in 4 s " +
                    "(energy ${before.energy} → ${after.energy}, level ${after.level}); this side's microphone, expected $expectedMicrophone, " +
                    "measured energy ${source.energy} and level ${source.level} at its source; " +
                    "the browser received ${browserAfter.first - browserBefore.first} packets and " +
                    "${browserAfter.second - browserBefore.second} samples in 4 s, peaking at $browserPeak; $hold; $video; " +
                    "resumed on socket 2 with session $session with video flowing on; camera off; the signalling socket negotiated ${sockets.latestExtensions}; left",
            )
        } finally {
            listening.cancel()
            call.audio.closeMicrophone()
            call.close()
        }
    }

    // --- Helpers the camera needs; the rest are `CallTestSupport`'s --------------------------

    /** Opens `camera` on `id` for `origin`'s engine, on the main thread, and answers the track and the camera opened. */
    private suspend fun cameraStarted(camera: CallCamera, id: String?, origin: String): Pair<TrackId, CallCameraDevice> =
        withContext(Dispatchers.Main) {
            suspendCoroutine { continuation ->
                camera.start(id, origin) { result ->
                    result.fold({ continuation.resume(it) }, { continuation.resumeWithException(it) })
                }
            }
        }

    /** A sample of `user`'s camera from the browser, waiting up to `seconds` for a frame. */
    private suspend fun untilSample(what: String, seconds: Long, helper: String, user: String): CameraSample {
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(seconds)
        while (true) {
            val answer = http("GET", "$helper/second-participant/camera-sample?user=$user")
            if (answer != "null") {
                val sample = JSONObject(answer)
                if (sample.optInt("width") > 0) {
                    return CameraSample(sample.getInt("width"), sample.getInt("height"), sample.getDouble("brightness"), sample.getDouble("spread"))
                }
            }
            if (System.nanoTime() > deadline) {
                fail("$what did not happen within $seconds s")
            }
            delay(500)
        }
    }
}
