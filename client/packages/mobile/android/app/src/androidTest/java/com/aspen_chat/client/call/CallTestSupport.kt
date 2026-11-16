package com.aspen_chat.client.call

import android.app.Activity
import android.app.ActivityManager
import android.app.NotificationManager
import android.content.Context
import android.os.ParcelFileDescriptor
import android.os.ParcelUuid
import android.os.Process
import android.service.notification.StatusBarNotification
import android.view.View
import android.widget.TextView
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import com.aspen_chat.client.push.WebPush
import com.aspen_chat.client.voice.CallState
import com.aspen_chat.client.voice.PeerConnection
import com.aspen_chat.client.voice.StatsOf
import java.math.BigInteger
import java.net.HttpURLConnection
import java.net.URL
import java.nio.charset.StandardCharsets
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.SecureRandom
import java.security.interfaces.ECPrivateKey
import java.security.interfaces.ECPublicKey
import java.security.spec.ECGenParameterSpec
import java.security.spec.ECPublicKeySpec
import java.util.Base64
import java.util.concurrent.TimeUnit
import javax.crypto.Cipher
import javax.crypto.KeyAgreement
import javax.crypto.spec.GCMParameterSpec
import javax.crypto.spec.SecretKeySpec
import kotlinx.coroutines.delay
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Assume.assumeTrue

/**
 * What the call tests share: the check's arguments, waiting on the engine and Telecom, the
 * system's own word read through the instrumentation's shell, the helper that drives the browser,
 * and the statistics each side is judged by.
 */
object CallTestSupport {
    /** What an `inbound-rtp` audio entry of a statistics report measured. */
    data class InboundAudio(val packets: Long, val energy: Double, val level: Double)

    /** What the microphone's `media-source` entry measured: `totalAudioEnergy` and `audioLevel`. */
    data class SourceAudio(val energy: Double, val level: Double)

    /** The instrumentation's argument `name`, or the test is skipped: the check gives it, nothing else does. */
    fun argument(name: String): String {
        val value = InstrumentationRegistry.getArguments().getString(name)
        assumeTrue("$name is not given", !value.isNullOrEmpty())
        return value!!
    }

    /** The output of `command` run through the instrumentation's shell, the system's own word. */
    fun shell(command: String): String {
        val descriptor = InstrumentationRegistry.getInstrumentation().uiAutomation.executeShellCommand(command)
        return ParcelFileDescriptor.AutoCloseInputStream(descriptor).bufferedReader().use { it.readText() }
    }

    /** The engine's state once `holds` of it, or a failure after `seconds`, naming the last state. */
    suspend fun until(what: String, seconds: Long, call: CallEngine, holds: (CallState) -> Boolean): CallState {
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
    suspend fun untilTelecom(what: String, seconds: Long, call: CallEngine, holds: (CallTelecom.Phase) -> Boolean): CallTelecom.Phase {
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
    suspend fun <T : Any> awaitValue(what: String, seconds: Long, read: () -> T?): T {
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
    suspend fun awaitTrue(what: String, seconds: Long, holds: () -> Boolean) {
        awaitValue(what, seconds) { if (holds()) Unit else null }
    }

    /** The call's service among the app's running services, with whether it is in the foreground. */
    @Suppress("DEPRECATION")
    fun callService(context: Context): ActivityManager.RunningServiceInfo? =
        context.getSystemService(ActivityManager::class.java).getRunningServices(100)
            .firstOrNull { it.service.className == CallService::class.java.name }

    fun callNotification(context: Context): StatusBarNotification? =
        context.getSystemService(NotificationManager::class.java).activeNotifications.firstOrNull { it.id == CallService.NOTIFICATION_ID }

    /** The lines of `dumpsys telecom` that name the call `id`, Telecom's own word on it. */
    fun telecomLines(id: ParcelUuid): List<String> = shell("dumpsys telecom").lines().filter { it.contains(id.toString()) }

    /** Whether the activity the system holds resumed is the incoming call screen, as `dumpsys activity` says. */
    fun incomingCallScreenShown(): Boolean =
        shell("dumpsys activity activities").lines().any { it.contains("ResumedActivity") && it.contains(IncomingCallActivity::class.java.simpleName) }

    /** The incoming call screen of this process while it is resumed, else `null`. */
    fun incomingCallScreen(): IncomingCallActivity? {
        var found: IncomingCallActivity? = null
        InstrumentationRegistry.getInstrumentation().runOnMainSync {
            found = ActivityLifecycleMonitorRegistry.getInstance().getActivitiesInStage(Stage.RESUMED).filterIsInstance<IncomingCallActivity>().firstOrNull()
        }
        return found
    }

    /** Taps the middle of the view `id` of `activity` through the system's input, as a finger on the screen would. */
    fun tap(activity: Activity, id: Int) {
        val view = activity.findViewById<View>(id)
        val at = IntArray(2)
        InstrumentationRegistry.getInstrumentation().runOnMainSync { view.getLocationOnScreen(at) }
        shell("input tap ${at[0] + view.width / 2} ${at[1] + view.height / 2}")
    }

    fun textOf(activity: Activity, id: Int): String {
        var text = ""
        InstrumentationRegistry.getInstrumentation().runOnMainSync { text = activity.findViewById<TextView>(id).text.toString() }
        return text
    }

    /**
     * The ringtone players of this process as the audio system lists them (`dumpsys audio`), each
     * line naming its state and whether the system mutes it (`mutedState`).
     */
    fun ringtonePlayers(): List<String> =
        shell("dumpsys audio").lines().filter {
            it.contains("u/pid:${Process.myUid()}/${Process.myPid()}") && it.contains("usage=USAGE_NOTIFICATION_RINGTONE")
        }.map { it.trim() }

    /** The ringtone player's state and mute, from its line: `started` and `none`, say. */
    fun playerState(line: String): Pair<String, String> =
        (Regex("state:(\\S+)").find(line)?.groupValues?.get(1) ?: "unknown") to (Regex("mutedState:(\\S+)").find(line)?.groupValues?.get(1) ?: "unknown")

    /**
     * Has the second participant call the device's user in their DM from the browser, and gives
     * the ring the deployment then holds for the user, as the `call` pointer carries it.
     */
    fun ringFromTheDeployment(helper: String): JSONObject = JSONObject(http("POST", "$helper/ring"))

    /** Whether the deployment still holds the DM's ring for the device's user. */
    fun ringStanding(helper: String): Boolean = !JSONObject(http("GET", "$helper/ring")).isNull("ring")

    /** Has the second participant leave the DM's call, which ends it, and the ring with it. */
    fun endCallElsewhere(helper: String) {
        http("POST", "$helper/ring/end")
    }

    /** The body of `url`'s answer, which must be `200`. */
    fun http(method: String, url: String): String {
        val connection = URL(url).openConnection() as HttpURLConnection
        connection.requestMethod = method
        connection.connectTimeout = 60_000
        connection.readTimeout = 60_000
        val status = connection.responseCode
        val body = (if (status < 400) connection.inputStream else connection.errorStream)?.bufferedReader()?.readText() ?: ""
        assertEquals("$method $url: $body", 200, status)
        return body
    }

    suspend fun entries(connection: PeerConnection): List<JSONObject> {
        val array = JSONArray(connection.stats(StatsOf.Connection))
        return (0 until array.length()).map { array.getJSONObject(it) }
    }

    /** The connection's inbound audio, summed over its `inbound-rtp` audio entries. */
    suspend fun inboundAudio(connection: PeerConnection): InboundAudio {
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
    suspend fun sourceAudio(connection: PeerConnection): SourceAudio {
        val source = entries(connection).firstOrNull { it.optString("type") == "media-source" && it.optString("kind") == "audio" }
        return SourceAudio(source?.optDouble("totalAudioEnergy", 0.0) ?: 0.0, source?.optDouble("audioLevel", 0.0) ?: 0.0)
    }

    /** Whom the browser's call shows on hold, from the helper's report of it. */
    fun browserOnHold(helper: String): List<String> {
        val onHold = JSONObject(http("GET", "$helper/second-participant/report")).getJSONArray("onHold")
        return (0 until onHold.length()).map { onHold.getString(it) }
    }

    /** The packets and samples the browser's connection received of audio, from the helper's report of it. */
    fun browserInbound(helper: String): Pair<Long, Long> {
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

    /**
     * The push state as the web view writes it on sign-in, holding one account at `origin`:
     * `userId` and `refreshToken` as given, a session token the deployment refuses
     * (`stale-session-token`), so the first request goes through the renewal, and `keys` the
     * phone's, which `pushes` encrypts to.
     */
    fun pushState(origin: String, subscription: String, userId: String, refreshToken: String, pushes: Pushes): String {
        val account = JSONObject()
            .put("subscription", subscription)
            .put("origin", origin)
            .put("userId", userId)
            .put("refreshToken", refreshToken)
            .put("sessionToken", "stale-session-token")
            .put("keys", pushes.keys())
            .put("applicationServerKey", "key")
            .put("deploymentSubscription", "dep")
        return JSONObject().put("version", 1).put("device", JSONObject.NULL).put("accounts", JSONArray().put(account)).toString()
    }

    /**
     * A phone's push keys, and pointers encrypted to them as a deployment encrypts them
     * (RFC 8291), for feeding the push handler what the messaging service would.
     */
    class Pushes {
        private val phonePublic: ByteArray
        private val phonePrivate: ECPrivateKey
        private val auth = ByteArray(16).also { SecureRandom().nextBytes(it) }

        init {
            val generator = KeyPairGenerator.getInstance("EC")
            generator.initialize(ECGenParameterSpec("secp256r1"))
            val phone = generator.generateKeyPair()
            phonePublic = point(phone.public as ECPublicKey)
            phonePrivate = phone.private as ECPrivateKey
        }

        /** The account's `keys` as the push state holds them. */
        fun keys(): JSONObject {
            val privateKey = JSONObject().put("kty", "EC").put("crv", "P-256").put("d", b64(unsigned(phonePrivate.s)))
            return JSONObject().put("privateKey", privateKey).put("publicKey", b64(phonePublic)).put("auth", b64(auth))
        }

        /** The pointer, encrypted to the phone, base64url. */
        fun encrypt(pointer: JSONObject): String {
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

        companion object {
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
}
