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
import com.aspen_chat.client.call.CallTestSupport.endCallElsewhere
import com.aspen_chat.client.call.CallTestSupport.http
import com.aspen_chat.client.call.CallTestSupport.inboundAudio
import com.aspen_chat.client.call.CallTestSupport.incomingCallScreen
import com.aspen_chat.client.call.CallTestSupport.incomingCallScreenShown
import com.aspen_chat.client.call.CallTestSupport.playerState
import com.aspen_chat.client.call.CallTestSupport.ringFromTheDeployment
import com.aspen_chat.client.call.CallTestSupport.ringStanding
import com.aspen_chat.client.call.CallTestSupport.ringtonePlayers
import com.aspen_chat.client.call.CallTestSupport.shell
import com.aspen_chat.client.call.CallTestSupport.tap
import com.aspen_chat.client.call.CallTestSupport.telecomLines
import com.aspen_chat.client.call.CallTestSupport.textOf
import com.aspen_chat.client.call.CallTestSupport.until
import com.aspen_chat.client.call.CallTestSupport.untilTelecom
import com.aspen_chat.client.R
import com.aspen_chat.client.push.PushHandler
import com.aspen_chat.client.push.PushStore
import com.aspen_chat.client.voice.CallStatus
import com.aspen_chat.client.voice.PeerConnection
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
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
 * Ringing: the second participant calls the user's DM from the browser, and the ring the deployment
 * holds is fed to the push handler as its push would be. With the screen off and the phone locked,
 * the incoming call's notification is up within five seconds with its full-screen intent, Telecom
 * holds the call as ringing, the screen turns on with the incoming call screen resumed over the lock
 * screen, and the ringtone plays as the phone's settings allow (muted by the system in Do Not
 * Disturb, heard otherwise, as `dumpsys audio` says of the player); the screen is captured lit
 * (`/data/local/tmp/aspen-ring-*.png`, which the check pulls). The screen's Decline ends the ring
 * at the deployment; its Answer joins the DM's call through the engine, the call going on in the
 * screen while the phone stays locked until its Hang up; the caller hanging up, Telecom's own end of
 * a ringing call (`KEYCODE_ENDCALL`, the phone's end key, which Telecom takes as the user
 * declining), and the notification's Decline each end the ring, the screen going with it; and a ring
 * under Doze is shown as fast, and wakes the phone.
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
    /** Whether the system itself sent the ring's full-screen intent, once a ring has shown. */
    private var fullScreenSentBySystem: Boolean? = null
    /** What the notification's record said of Do Not Disturb as the last ring was shown. */
    private var recordState: String? = null

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

    /**
     * The record in `dumpsys notification` of the notification the system gave `key`
     * (`StatusBarNotification.key`, which the record line carries whole), or `null` while the
     * dump lists none; `dumpLines` is what the dump last named of the package, for a failure to
     * quote.
     */
    private var dumpLines: List<String> = emptyList()

    private fun notificationRecordOf(key: String): String? {
        val lines = shell("dumpsys notification --noredact").lines()
        dumpLines = lines.filter { it.contains(context.packageName) }.map { it.trim() }
        return lines.firstOrNull { it.contains("NotificationRecord(") && it.contains("key=$key") }?.trim()
    }

    /**
     * The call notification's record in `dumpsys notification`'s posted list (`Notification List`)
     * with the lines of its state (`mIntercept`, `mSuppressedVisualEffects`, and the rest), or
     * `null` while the system lists none there: a notification just posted sits in the enqueued
     * list first, with no Do Not Disturb decision yet, and the dump names it in its history too.
     */
    private fun notificationRecordState(): List<String>? {
        val lines = shell("dumpsys notification --noredact").lines()
        val posted = lines.indexOfFirst { it.trim() == "Notification List:" }
        if (posted < 0) return null
        // The section ends at the next heading, a line indented like it and ending in a colon.
        val indent = lines[posted].takeWhile { it == ' ' }
        val section = lines.drop(posted + 1).takeWhile { !(it.startsWith(indent) && !it.startsWith("$indent ") && it.trimEnd().endsWith(":")) }
        for ((index, line) in section.withIndex()) {
            if (line.contains("NotificationRecord(") && line.contains("pkg=${context.packageName}") && line.contains("id=${CallService.NOTIFICATION_ID}")) {
                return section.drop(index + 1).takeWhile { !it.contains("NotificationRecord(") }.map { it.trim() }
            }
        }
        return null
    }

    /**
     * Whether Do Not Disturb intercepts the call's notification and keeps its full-screen intent
     * from being sent (the phone's own Don't turn on screen), from its record's `mIntercept` and
     * `mSuppressedVisualEffects`; `null` while the system lists no record.
     */
    private fun fullScreenIntentSuppressed(): Boolean? {
        val record = notificationRecordState() ?: return null
        val intercepted = record.any { it == "mIntercept=true" }
        val effects = record.firstNotNullOfOrNull { Regex("mSuppressedVisualEffects= ?(\\d+)").find(it)?.groupValues?.get(1)?.toInt() } ?: 0
        return intercepted && (effects and NotificationManager.Policy.SUPPRESSED_EFFECT_FULL_SCREEN_INTENT) != 0
    }

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
        val call = CallEngine(context, CallAccount(api, user), loopbackPlainText = true, media = media, peerConnections = peerConnections)
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
            val posted = awaitValue("the app sees the call's notification posted", 10) { callNotification(context) }
            val record = try {
                awaitValue("the system's dump lists the call's notification ${posted.key}", 10) { notificationRecordOf(posted.key) }
            } catch (missing: AssertionError) {
                throw AssertionError("${missing.message}; the dump last named the package in: $dumpLines", missing)
            }
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
            awaitTrue("the notification is gone from the system's list", 10) { callNotification(context) == null && notificationRecordOf(posted.key) == null }
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
        val user = argument("ASPEN_CALL_USER_ID")
        val refreshToken = argument("ASPEN_CALL_REFRESH_TOKEN")
        val helper = argument("ASPEN_CALL_HELPER")
        this@CallDeviceTest.helper = helper
        val pushes = CallTestSupport.Pushes()
        val subscription = "call-device-test-ring"
        store.save(CallTestSupport.pushState(api, subscription, user, refreshToken, pushes))
        val engine = CallEngines.forAccount(context, CallAccount(api, user))
        assertEquals("nothing rings before", CallTelecom.Phase.IDLE, engine.telecom.phase)
        val declineIntent = CallService.declineIntent(context)
        val filter = notifications.currentInterruptionFilter
        val doNotDisturb = filter != NotificationManager.INTERRUPTION_FILTER_ALL

        /** Bob calls the DM from the browser, and the ring the deployment holds is fed to the push handler as its push would be. */
        fun ring(): Ring {
            val held = ringFromTheDeployment(helper)
            val pointer = JSONObject()
                .put("v", 1)
                .put("kind", "call")
                .put("channel", held.getString("channel"))
                .put("session", held.getString("session"))
                .put("caller", held.getString("caller"))
                .put("callerName", held.getString("callerName"))
                .put("until", held.getString("until"))
            val handled = System.nanoTime()
            PushHandler(context).handle(subscription, pushes.encrypt(pointer))
            val shown = runBlocking { awaitValue("the incoming call's notification", 5) { callNotification(context)?.notification } }
            return Ring(shown, (System.nanoTime() - handled) / 1_000_000, held.getString("channel"), held.getString("callerName"))
        }

        /**
         * The ring is shown: the screen on with the incoming call screen resumed, and the
         * ringtone playing as the system allows. The system sends the full-screen intent itself
         * unless the phone's Do Not Disturb intercepts the notification and keeps the screen dark
         * (`dark` says whether it was); then the test sends it, as the system would on a phone
         * whose settings let it, and the screen must still turn on over the keyguard.
         */
        suspend fun shown(ring: Ring, dark: Boolean): IncomingCallActivity {
            assertNotNull("the notification can wake the lock screen", ring.notification.fullScreenIntent)
            assertEquals(Notification.CATEGORY_CALL, ring.notification.category)
            untilTelecom("Telecom holds the call as ringing", 5, engine) { it == CallTelecom.Phase.RINGING }
            val suppressed = awaitValue("the notification's record", 5) { fullScreenIntentSuppressed() }
            recordState = notificationRecordState()?.filter { it.startsWith("mIntercept=") || it.startsWith("mSuppressedVisualEffects=") }?.joinToString(" ")
            if (suppressed) {
                if (dark) {
                    delay(3_000)
                    assertTrue("Do Not Disturb keeps the screen dark, as the phone asks (${wakefulness()})", wakefulness() != "Awake")
                }
                ring.notification.fullScreenIntent.send()
            }
            fullScreenSentBySystem = fullScreenSentBySystem ?: !suppressed
            awaitTrue("the screen turns on", 5) { wakefulness() == "Awake" }
            awaitTrue("the incoming call screen is shown", 5) { incomingCallScreenShown() }
            val screen = awaitValue("the incoming call screen is this process's", 5) { incomingCallScreen() }
            assertEquals(ring.callerName, textOf(screen, R.id.incoming_call_caller))
            val player = awaitValue("the ringtone plays", 5) {
                ringtonePlayers().firstOrNull { playerState(it).first == "started" }
            }
            val (_, muted) = playerState(player)
            if (doNotDisturb) {
                assertTrue("in Do Not Disturb (filter $filter) the system mutes the ringtone: $player", muted != "none")
            } else {
                assertEquals("with interruptions allowed the ringtone is heard: $player", "none", muted)
            }
            return screen
        }

        suspend fun gone(what: String) {
            untilTelecom("$what ends the ring", 10, engine) { it == CallTelecom.Phase.IDLE }
            awaitTrue("and takes the notification down", 10) { callNotification(context) == null }
            awaitTrue("and closes the incoming call screen", 10) { !incomingCallScreenShown() && incomingCallScreen() == null }
            awaitTrue("and stops the ringtone", 5) { ringtonePlayers().none { playerState(it).first == "started" } }
        }

        // Dark and locked: the ring turns the screen on and shows the incoming call screen over
        // the lock screen, ringing as the phone's settings allow, captured lit; its Decline ends
        // the ring at the deployment.
        shell("input keyevent KEYCODE_SLEEP")
        awaitTrue("the screen goes off", 10) { wakefulness() != "Awake" }
        val dark = ring()
        assertTrue("posted within five seconds (took ${dark.millis} ms)", dark.millis < 5_000)
        val darkScreen = shown(dark, dark = true)
        val fullScreenAllowed = notifications.canUseFullScreenIntent()
        delay(1_500)
        shell("screencap -p /data/local/tmp/aspen-ring-dark.png")
        val darkPlayer = ringtonePlayers().joinToString("; ")
        val sentBy = if (fullScreenSentBySystem == true) "the system sent the full-screen intent" else "Do Not Disturb kept the system from sending the full-screen intent ($recordState), so the test sent it"
        println("call-device-test: rang with the screen dark in ${dark.millis} ms; canUseFullScreenIntent $fullScreenAllowed; $sentBy; the screen is ${wakefulness()}, ${keyguardShowing()}, the incoming call screen shown; ringtone player: $darkPlayer (captured /data/local/tmp/aspen-ring-dark.png)")
        tap(darkScreen, R.id.incoming_call_decline)
        gone("the screen's Decline")
        awaitTrue("the deployment no longer holds the ring", 10) { !ringStanding(helper) }
        println("call-device-test: the screen's Decline ended the ring; the deployment holds no ring for the user")

        // Lit and locked: the screen shows over the lock screen; its Answer joins the DM's call,
        // and the call goes on there, with the phone still locked, until Hang up.
        val lit = ring()
        assertTrue("posted within five seconds (took ${lit.millis} ms)", lit.millis < 5_000)
        val litScreen = shown(lit, dark = false)
        val ringingId = awaitValue("Telecom's id for the call", 5) { engine.telecom.callId }
        val ringingLines = awaitValue("Telecom lists the call ringing", 5) {
            telecomLines(ringingId).filter { it.contains("RINGING") }.takeIf { it.isNotEmpty() }
        }
        delay(1_000)
        shell("screencap -p /data/local/tmp/aspen-ring-lock-screen.png")
        println("call-device-test: rang with the screen lit in ${lit.millis} ms, ${keyguardShowing()}; Telecom: ${ringingLines.first().trim()}; captured /data/local/tmp/aspen-ring-lock-screen.png")
        tap(litScreen, R.id.incoming_call_answer)
        until("the engine joins the DM's call", 40, engine) { it.status == CallStatus.CONNECTED && it.channel == lit.channel }
        untilTelecom("the system call is active after answering", 20, engine) { it == CallTelecom.Phase.ACTIVE }
        val answeredLines = telecomLines(ringingId).filter { it.contains("ACTIVE") }
        assertTrue("Telecom holds the answered call active: $answeredLines", answeredLines.isNotEmpty())
        val ongoing = awaitValue("the ongoing call's notification replaces the incoming one", 10) {
            callNotification(context)?.notification?.takeIf { it.fullScreenIntent == null }
        }
        assertEquals(Notification.CATEGORY_CALL, ongoing.category)
        awaitTrue("the ringtone stops on answering", 5) { ringtonePlayers().none { playerState(it).first == "started" } }
        assertTrue("the screen stays as the call's while the phone is locked", incomingCallScreenShown())
        awaitTrue("and shows the call under way", 10) { textOf(litScreen, R.id.incoming_call_status) == context.getString(R.string.aspen_call_in_progress) }
        shell("screencap -p /data/local/tmp/aspen-ring-answered-locked.png")
        println("call-device-test: answered from the screen; the engine joined ${lit.channel} and Telecom holds it: ${answeredLines.first().trim()}; the screen shows the call on the locked phone (captured /data/local/tmp/aspen-ring-answered-locked.png)")
        tap(litScreen, R.id.incoming_call_hang_up)
        until("the screen's Hang up leaves the call", 10, engine) { it.status == CallStatus.IDLE }
        untilTelecom("and ends the system call", 20, engine) { it == CallTelecom.Phase.IDLE }
        awaitTrue("and takes the notification down", 10) { callNotification(context) == null }
        awaitTrue("and closes the screen", 10) { !incomingCallScreenShown() }
        endCallElsewhere(helper)
        println("call-device-test: the screen's Hang up left the call")

        // The call ending elsewhere, its caller hanging up: the ring, the notification, and the
        // screen go, as the engine's watch hears of it.
        val abandoned = ring()
        shown(abandoned, dark = false)
        endCallElsewhere(helper)
        gone("the caller hanging up")
        println("call-device-test: the caller hanging up ended the ring and closed the screen")

        // Decline, from Telecom itself: the phone's end key ends a ringing call as the user's
        // decline, which reaches the layer as Telecom's disconnect while ringing.
        val byKey = ring()
        shown(byKey, dark = false)
        val declinedId = awaitValue("Telecom's id for the call", 5) { engine.telecom.callId }
        shell("input keyevent KEYCODE_ENDCALL")
        gone("the end key")
        awaitTrue("the deployment no longer holds the ring", 10) { !ringStanding(helper) }
        assertFalse("the ring is declined, not answered", engine.engine.state().status == CallStatus.CONNECTED)
        val declinedLines = telecomLines(declinedId)
        assertTrue("Telecom no longer lists the call ringing: $declinedLines", declinedLines.none { it.contains("RINGING") })
        println("call-device-test: the end key declined the ring (${byKey.notification.extras.getCharSequence(Notification.EXTRA_TITLE)}); Telecom's call ended and the notification went")

        // And the notification's own Decline, as a tap on it.
        val byTap = ring()
        shown(byTap, dark = false)
        byTap.notification.actions.single { it.actionIntent == declineIntent }.actionIntent.send()
        gone("the notification's Decline")
        awaitTrue("the deployment no longer holds the ring", 10) { !ringStanding(helper) }

        // Under Doze: the ring is shown as fast, and wakes the phone.
        shell("input keyevent KEYCODE_SLEEP")
        awaitTrue("the screen goes off", 10) { wakefulness() != "Awake" }
        shell("dumpsys deviceidle force-idle")
        awaitValue("Doze is forced", 10) { dozeState().takeIf { it == "IDLE" } }
        val dozed = ring()
        assertTrue("posted within five seconds under Doze (took ${dozed.millis} ms)", dozed.millis < 5_000)
        shown(dozed, dark = true)
        shell("dumpsys deviceidle unforce")
        dozed.notification.actions.single { it.actionIntent == declineIntent }.actionIntent.send()
        gone("the notification's Decline under Doze")
        println("call-device-test: rang under Doze in ${dozed.millis} ms, waking the screen; declined from the notification; Doze ${dozeState()} again")
    }

    /** One ring fed to the push handler: its notification, how long it took to post, and whose call it is. */
    private class Ring(val notification: Notification, val millis: Long, val channel: String, val callerName: String)
}
