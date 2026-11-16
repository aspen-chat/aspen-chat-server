package com.aspen_chat.client.call

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import android.content.Context
import android.media.AudioManager
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.aspen_chat.client.call.CallTestSupport.argument
import com.aspen_chat.client.call.CallTestSupport.awaitTrue
import com.aspen_chat.client.call.CallTestSupport.awaitValue
import com.aspen_chat.client.call.CallTestSupport.browserInbound
import com.aspen_chat.client.call.CallTestSupport.callNotification
import com.aspen_chat.client.call.CallTestSupport.http
import com.aspen_chat.client.call.CallTestSupport.inboundAudio
import com.aspen_chat.client.call.CallTestSupport.shell
import com.aspen_chat.client.call.CallTestSupport.telecomLines
import com.aspen_chat.client.call.CallTestSupport.until
import com.aspen_chat.client.call.CallTestSupport.untilTelecom
import com.aspen_chat.client.push.PushHandler
import com.aspen_chat.client.push.PushStore
import com.aspen_chat.client.voice.CallStatus
import com.aspen_chat.client.voice.PeerConnection
import java.time.Instant
import java.util.UUID
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

/**
 * What only a phone can show of a call, read from the system itself through the instrumentation's
 * shell (`dumpsys audio`, `power`, `activity services`, `notification`, `telecom`, and
 * `deviceidle`), against the same deployment and browser as `CallEngineTest`, with the same
 * arguments (`ASPEN_CALL_*`); it is skipped without them and on an emulator, which has no
 * earpiece and no lock screen anyone sees.
 *
 * In a call: the earpiece and the loudspeaker are Telecom's endpoints and selecting one moves the
 * sound there, as the audio system's active communication device says; the proximity wake lock is
 * held at the ear and released at the loudspeaker; the call's service runs in the foreground with
 * its `CallStyle` notification; the call stays connected with the screen off for three minutes and
 * then under Doze forced on, media flowing both ways throughout; and leaving takes it all down.
 *
 * Ringing: a `call` push with the screen off and the phone locked has the incoming call's
 * notification up within five seconds, its full-screen intent set, with Telecom holding the call
 * as ringing, whether it woke the screen is recorded, and the lock screen, lit, is captured
 * showing the call (`/data/local/tmp/aspen-ring-*.png`, which the check pulls); the
 * notification's Answer joins the DM's call through the engine, Telecom's own end of a ringing
 * call (`KEYCODE_ENDCALL`, the phone's end key, which Telecom takes as the user declining) and
 * the notification's Decline each end the ring; and a ring under Doze is shown as fast.
 *
 * The test process is the app's, instrumented, which keeps it alive through the screen off and
 * Doze; what the system would do to a process it does not hold is a device check with the app
 * itself (`android-builds.md`, What only a phone can show).
 */
@RunWith(AndroidJUnit4::class)
class CallDeviceTest {
    private lateinit var context: Context
    private lateinit var store: PushStore
    private lateinit var notifications: NotificationManager
    private var keptPushState: String? = null
    private var helper: String? = null

    /** The lines of `dumpsys` that say where the call plays and what wake locks are held. */
    private fun activeCommunicationDevice(): String =
        shell("dumpsys audio").lines().firstOrNull { it.contains("Active communication device:") }?.trim() ?: "none listed"

    private fun proximityLockHeld(): Boolean =
        shell("dumpsys power").lines().any { it.contains("PROXIMITY_SCREEN_OFF_WAKE_LOCK") && it.contains("'aspen:call'") }

    private fun wakefulness(): String =
        shell("dumpsys power").lines().firstOrNull { it.trim().startsWith("mWakefulness=") }?.trim()?.removePrefix("mWakefulness=") ?: "unknown"

    private fun dozeState(): String = shell("dumpsys deviceidle get deep").trim()

    /** Whether the lock screen is up, as the window manager says; a phone unlocked by its owner's face meanwhile shows none. */
    private fun keyguardShowing(): String =
        shell("dumpsys window").lines().firstOrNull { it.contains("isKeyguardShowing=") }?.trim() ?: "isKeyguardShowing unknown"

    /** The call service's line in `dumpsys activity services`, with its foreground state and types. */
    private fun serviceLines(): List<String> =
        shell("dumpsys activity services ${context.packageName}").lines().filter {
            it.contains("isForeground=") || it.contains("foregroundServiceType=") || it.contains(CallService::class.java.name)
        }.map { it.trim() }

    /** The call notification's record in `dumpsys notification`, by package and id. */
    private fun notificationRecord(): String? =
        shell("dumpsys notification --noredact").lines().firstOrNull {
            it.contains("pkg=${context.packageName}") && it.contains("id=${CallService.NOTIFICATION_ID}")
        }?.trim()

    @Before
    fun setUp() {
        context = InstrumentationRegistry.getInstrumentation().targetContext
        store = PushStore(context)
        notifications = context.getSystemService(NotificationManager::class.java)
        val automation = InstrumentationRegistry.getInstrumentation().uiAutomation
        automation.grantRuntimePermission(context.packageName, Manifest.permission.RECORD_AUDIO)
        automation.grantRuntimePermission(context.packageName, Manifest.permission.POST_NOTIFICATIONS)
        // An emulator's kernel says so; a phone's does not.
        assumeTrue("a phone, not an emulator", shell("getprop ro.kernel.qemu").trim() != "1")
        keptPushState = store.load()
    }

    @After
    fun tearDown() {
        // Whatever the test changed of the phone is given back: Doze unforced, the screen awake,
        // the notifications down, the push state as it was.
        shell("dumpsys deviceidle unforce")
        shell("input keyevent KEYCODE_WAKEUP")
        notifications.cancelAll()
        val kept = keptPushState
        if (kept != null) store.save(kept) else store.remove()
        helper?.let { http("POST", "$it/second-participant/leave") }
    }

    @Test
    fun playsAtTheEarAndStaysConnectedThroughTheScreenOffAndDoze() = runBlocking {
        val api = argument("ASPEN_CALL_API")
        val channel = argument("ASPEN_CALL_CHANNEL")
        val user = argument("ASPEN_CALL_USER_ID")
        val refreshToken = argument("ASPEN_CALL_REFRESH_TOKEN")
        val helper = argument("ASPEN_CALL_HELPER")
        this@CallDeviceTest.helper = helper
        store.save(CallTestSupport.pushState(api, "call-device-test", user, refreshToken, CallTestSupport.Pushes()))

        val media = CallEngine.peerConnections(context)
        val peerConnections = CallEngineTest.RecordingPeerConnections(media)
        val call = CallEngine(context, api, loopbackPlainText = true, media = media, peerConnections = peerConnections)
        val audioManager = context.getSystemService(AudioManager::class.java)
        try {
            assumeTrue("a phone with an earpiece", call.routes.hasEarpiece)
            call.engine.setMicrophone(call.audio.openMicrophone())
            call.engine.join(channel)
            until("the call connects", 40, call) { it.status == CallStatus.CONNECTED }
            untilTelecom("the system call is active", 20, call) { it == CallTelecom.Phase.ACTIVE }
            val callId = call.telecom.callId ?: throw AssertionError("Telecom gave the call no id")
            val connection = peerConnections.latest ?: throw AssertionError("no peer connection")
            val bob = JSONObject(http("POST", "$helper/second-participant")).getString("user")
            until("the second participant takes a slot", 30, call) { state -> state.slots.any { it.user == bob && it.track != null } }

            // The earpiece: Telecom's endpoint, the audio system's active device, and the
            // proximity wake lock held for as long as the call plays there.
            awaitTrue("Telecom lists the earpiece", 10) { call.routes.routes.any { it.kind == CallRoutes.Kind.EARPIECE } }
            assertTrue(call.routes.select(CallRoutes.Kind.EARPIECE))
            awaitTrue("the call moves to the earpiece", 10) { call.routes.currentKind == CallRoutes.Kind.EARPIECE }
            val atEar = awaitValue("the audio system plays through the earpiece", 10) {
                activeCommunicationDevice().takeIf { it.contains("type:earpiece") }
            }
            awaitTrue("the proximity wake lock is held at the ear", 10) { proximityLockHeld() }
            assertEquals(AudioManager.MODE_IN_COMMUNICATION, audioManager.mode)

            // The loudspeaker: the device follows, and the lock is released.
            assertTrue(call.routes.select(CallRoutes.Kind.SPEAKER))
            awaitTrue("the call moves to the loudspeaker", 10) { call.routes.currentKind == CallRoutes.Kind.SPEAKER }
            val atSpeaker = awaitValue("the audio system plays through the loudspeaker", 10) {
                activeCommunicationDevice().takeIf { it.contains("type:speaker") }
            }
            awaitTrue("the proximity wake lock is released at the loudspeaker", 10) { !proximityLockHeld() }
            val telecomRouted = telecomLines(callId).filter { it.contains("ACTIVE") }
            assertTrue("Telecom holds the call active: $telecomRouted", telecomRouted.isNotEmpty())
            println("call-device-test: routes ${call.routes.routes.map { "${it.kind}(${it.name})" }}; at the earpiece \"$atEar\" with the proximity lock held; at the loudspeaker \"$atSpeaker\" with it released")

            // The foreground service, typed as a phone call and the microphone, and the
            // notification, a `CallStyle`, as the system lists them.
            val service = awaitValue("the call's service runs in the foreground", 10) {
                serviceLines().takeIf { lines -> lines.any { it.contains("isForeground=true") } }
            }
            val record = awaitValue("the call's notification is listed", 10) { notificationRecord() }
            assertTrue("a CallStyle notification: $record", record.contains("CallStyle"))
            println("call-device-test: service ${service.filter { it.contains("isForeground=") || it.contains("foregroundServiceType=") }}; notification $record")

            // The screen off for three minutes: the call stays connected and media flows both
            // ways, read every half minute.
            shell("input keyevent KEYCODE_SLEEP")
            awaitTrue("the screen goes off", 10) { wakefulness() != "Awake" }
            val asleepAs = wakefulness()
            val flowed = flowFor(call, connection, helper, rounds = 6, secondsEach = 30)
            println("call-device-test: screen $asleepAs for 3 min: connected throughout, ${flowed.first} packets reached the phone and ${flowed.second} the browser")

            // Doze forced on (which this phone allows while charging): the call goes on.
            shell("dumpsys deviceidle force-idle")
            val doze = awaitValue("Doze is forced", 10) { dozeState().takeIf { it == "IDLE" } }
            val dozed = flowFor(call, connection, helper, rounds = 2, secondsEach = 15)
            shell("dumpsys deviceidle unforce")
            awaitTrue("Doze is lifted", 10) { dozeState() != "IDLE" }
            shell("input keyevent KEYCODE_WAKEUP")
            awaitTrue("the screen comes back", 10) { wakefulness() == "Awake" }
            println("call-device-test: Doze $doze for 30 s: connected throughout, ${dozed.first} packets reached the phone and ${dozed.second} the browser; screen ${wakefulness()} again")

            // Leaving: the system lists nothing of the call any more.
            call.engine.leave()
            until("the call is left", 10, call) { it.status == CallStatus.IDLE }
            untilTelecom("the system call ends", 20, call) { it == CallTelecom.Phase.IDLE }
            awaitTrue("the service leaves the foreground", 10) { serviceLines().none { it.contains("isForeground=true") } }
            awaitTrue("the notification is gone from the system's list", 10) { notificationRecord() == null }
            awaitTrue("the proximity wake lock is not held", 10) { !proximityLockHeld() }
            val telecomAfter = telecomLines(callId)
            assertTrue("Telecom no longer holds the call:\n$telecomAfter", telecomAfter.none { it.contains("ACTIVE") || it.contains("DIALING") })
            println("call-device-test: left; the service, the notification, and the wake lock are gone, and Telecom's call ended")
        } finally {
            call.audio.closeMicrophone()
            call.close()
        }
    }

    /**
     * `rounds` of `secondsEach`, each checking the call is connected and packets reached both
     * sides since the round began; the packets counted over all of them.
     */
    private suspend fun flowFor(call: CallEngine, connection: PeerConnection, helper: String, rounds: Int, secondsEach: Long): Pair<Long, Long> {
        var phone = inboundAudio(connection).packets
        var browser = browserInbound(helper).first
        val phoneAtStart = phone
        val browserAtStart = browser
        repeat(rounds) { round ->
            delay(secondsEach * 1_000)
            val state = call.engine.state()
            assertEquals("connected in round ${round + 1}: $state", CallStatus.CONNECTED, state.status)
            val phoneNow = inboundAudio(connection).packets
            val browserNow = browserInbound(helper).first
            assertTrue("packets reach the phone in round ${round + 1}: $phone then $phoneNow", phoneNow > phone + 100)
            // The microphone's Opus uses DTX, so a quiet room sends a few packets a second.
            assertTrue("packets reach the browser in round ${round + 1}: $browser then $browserNow", browserNow > browser)
            phone = phoneNow
            browser = browserNow
        }
        return (phone - phoneAtStart) to (browser - browserAtStart)
    }

    @Test
    fun ringsOnTheLockedScreenAndIsAnsweredAndDeclined() = runBlocking {
        val api = argument("ASPEN_CALL_API")
        val channel = argument("ASPEN_CALL_CHANNEL")
        val user = argument("ASPEN_CALL_USER_ID")
        val refreshToken = argument("ASPEN_CALL_REFRESH_TOKEN")
        this@CallDeviceTest.helper = argument("ASPEN_CALL_HELPER")
        val pushes = CallTestSupport.Pushes()
        val subscription = "call-device-test-ring"
        store.save(CallTestSupport.pushState(api, subscription, user, refreshToken, pushes))
        val engine = CallEngines.forOrigin(context, api)
        assertEquals("nothing rings before", CallTelecom.Phase.IDLE, engine.telecom.phase)
        val answerIntent = CallService.answerIntent(context)
        val declineIntent = CallService.declineIntent(context)

        fun ring(): Pair<Notification, Long> {
            val pointer = JSONObject()
                .put("v", 1)
                .put("kind", "call")
                .put("channel", channel)
                .put("session", UUID.randomUUID().toString())
                .put("caller", "0190f0a0-0000-7000-8000-000000000002")
                .put("callerName", "Alice")
                .put("until", Instant.now().plusSeconds(60).toString())
            val handled = System.nanoTime()
            PushHandler(context).handle(subscription, pushes.encrypt(pointer))
            val shown = runBlocking { awaitValue("the incoming call's notification", 5) { callNotification(context)?.notification } }
            return shown to (System.nanoTime() - handled) / 1_000_000
        }

        // Dark and locked: the ring, with whether the full-screen intent woke the screen, which
        // it does only for an activity that turns the screen on and shows over the keyguard; the
        // screen is captured as it is (black while it stays dark).
        shell("input keyevent KEYCODE_SLEEP")
        awaitTrue("the screen goes off", 10) { wakefulness() != "Awake" }
        val (dark, darkMillis) = ring()
        assertTrue("posted within five seconds (took $darkMillis ms)", darkMillis < 5_000)
        assertNotNull("the notification can wake the lock screen", dark.fullScreenIntent)
        assertEquals(Notification.CATEGORY_CALL, dark.category)
        untilTelecom("Telecom holds the call as ringing", 5, engine) { it == CallTelecom.Phase.RINGING }
        delay(3_000)
        val screenWhileDark = wakefulness()
        shell("screencap -p /data/local/tmp/aspen-ring-dark.png")
        val fullScreenAllowed = notifications.canUseFullScreenIntent()
        println("call-device-test: rang with the screen dark in $darkMillis ms; canUseFullScreenIntent $fullScreenAllowed; 3 s on, the screen is $screenWhileDark, ${keyguardShowing()} (captured /data/local/tmp/aspen-ring-dark.png)")
        shell("input keyevent KEYCODE_ENDCALL")
        untilTelecom("the end key declines the ring on the dark screen", 10, engine) { it == CallTelecom.Phase.IDLE }
        awaitTrue("and takes the notification down", 10) { callNotification(context) == null }

        // Lit and locked: the lock screen shows the call as it rings, captured for the record.
        shell("input keyevent KEYCODE_WAKEUP")
        awaitTrue("the screen comes on", 10) { wakefulness() == "Awake" }
        val (shown, afterMillis) = ring()
        assertTrue("posted within five seconds (took $afterMillis ms)", afterMillis < 5_000)
        untilTelecom("Telecom holds the call as ringing on the lock screen", 5, engine) { it == CallTelecom.Phase.RINGING }
        val ringingId = awaitValue("Telecom's id for the call", 5) { engine.telecom.callId }
        val ringingLines = awaitValue("Telecom lists the call ringing", 5) {
            telecomLines(ringingId).filter { it.contains("RINGING") }.takeIf { it.isNotEmpty() }
        }
        delay(1_500)
        shell("screencap -p /data/local/tmp/aspen-ring-lock-screen.png")
        println("call-device-test: rang with the screen lit in $afterMillis ms, ${keyguardShowing()}; Telecom: ${ringingLines.first().trim()}; captured /data/local/tmp/aspen-ring-lock-screen.png")

        // Answer, as a tap on the notification's Answer does: the engine joins the DM's call.
        val answer = shown.actions.single { it.actionIntent == answerIntent }
        answer.actionIntent.send()
        until("the engine joins the ring's channel", 40, engine) { it.status == CallStatus.CONNECTED && it.channel == channel }
        untilTelecom("the system call is active after answering", 20, engine) { it == CallTelecom.Phase.ACTIVE }
        val answeredLines = telecomLines(ringingId).filter { it.contains("ACTIVE") }
        assertTrue("Telecom holds the answered call active: $answeredLines", answeredLines.isNotEmpty())
        val ongoing = awaitValue("the ongoing call's notification replaces the incoming one", 10) {
            callNotification(context)?.notification?.takeIf { it.fullScreenIntent == null }
        }
        assertEquals(Notification.CATEGORY_CALL, ongoing.category)
        println("call-device-test: answered from the notification; the engine joined $channel and Telecom holds it: ${answeredLines.first().trim()}")
        engine.engine.leave()
        untilTelecom("the answered call ends", 20, engine) { it == CallTelecom.Phase.IDLE }
        awaitTrue("the notification is taken down", 10) { callNotification(context) == null }

        // Decline, from Telecom itself: the phone's end key ends a ringing call as the user's
        // decline, which reaches the layer as Telecom's disconnect while ringing.
        val (declinable, _) = ring()
        untilTelecom("Telecom holds the second call as ringing", 5, engine) { it == CallTelecom.Phase.RINGING }
        val declinedId = awaitValue("Telecom's id for the second call", 5) { engine.telecom.callId }
        shell("input keyevent KEYCODE_ENDCALL")
        untilTelecom("the end key declines the ring", 10, engine) { it == CallTelecom.Phase.IDLE }
        awaitTrue("and takes the notification down", 10) { callNotification(context) == null }
        assertFalse("the ring is declined, not answered", engine.engine.state().status == CallStatus.CONNECTED)
        val declinedLines = telecomLines(declinedId)
        assertTrue("Telecom no longer lists the call ringing: $declinedLines", declinedLines.none { it.contains("RINGING") })
        println("call-device-test: the end key declined the second ring (${declinable.extras.getCharSequence(Notification.EXTRA_TITLE)}); Telecom's call ended and the notification went")

        // And the notification's own Decline, as a tap on it.
        val (declinedByTap, _) = ring()
        untilTelecom("Telecom holds the third call as ringing", 5, engine) { it == CallTelecom.Phase.RINGING }
        declinedByTap.actions.single { it.actionIntent == declineIntent }.actionIntent.send()
        untilTelecom("the notification's Decline ends the ring", 10, engine) { it == CallTelecom.Phase.IDLE }
        awaitTrue("and takes the notification down", 10) { callNotification(context) == null }

        // Under Doze: the ring is shown as fast.
        shell("dumpsys deviceidle force-idle")
        awaitValue("Doze is forced", 10) { dozeState().takeIf { it == "IDLE" } }
        val (dozed, dozedMillis) = ring()
        assertTrue("posted within five seconds under Doze (took $dozedMillis ms)", dozedMillis < 5_000)
        untilTelecom("Telecom holds the call as ringing under Doze", 5, engine) { it == CallTelecom.Phase.RINGING }
        shell("dumpsys deviceidle unforce")
        dozed.actions.single { it.actionIntent == declineIntent }.actionIntent.send()
        untilTelecom("the ring under Doze is declined", 10, engine) { it == CallTelecom.Phase.IDLE }
        assertNull(callNotification(context))
        println("call-device-test: rang under Doze in $dozedMillis ms, declined from the notification; Doze ${dozeState()} again")
    }
}
