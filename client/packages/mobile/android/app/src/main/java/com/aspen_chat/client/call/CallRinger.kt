package com.aspen_chat.client.call

import android.content.Context
import android.media.AudioAttributes
import android.media.Ringtone
import android.media.RingtoneManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.VibrationAttributes
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.util.Log

/**
 * Makes an incoming call heard and felt. Telecom plays no ringtone for a call an app manages
 * itself, as `CallsManager`'s are (the phone's own ring is its dialer's: through every ring of the
 * device proof the only ringtone player the audio system opened was this one), and a
 * notification's sound plays once, so the ring is this: the system's default ringtone (Aspen has
 * none of its own), looped, on the ring stream, with the ring vibration pattern, from `start`
 * until `stop`.
 *
 * Whether either is heard is the system's decision, made where it is made for the phone's own
 * calls: both are played as the ringtone usage (`USAGE_NOTIFICATION_RINGTONE`, `USAGE_RINGTONE`),
 * which the ringer mode silences (silent and vibrate mute the ring stream; silent stops ring
 * vibrations too), Do Not Disturb suppresses by its own rules for calls (the audio and vibrator
 * services apply them to that usage, as `dumpsys audio` shows of a muted player), and the Vibrate
 * for calls setting turns off. Nothing here reads those settings itself: a second reading of them
 * would disagree with the system's the day either changed.
 */
class CallRinger(context: Context) {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private var ringtone: Ringtone? = null
    private var vibrator: Vibrator? = null

    /** Whether `start` has been called and `stop` has not. */
    val ringing: Boolean
        get() = ringtone != null || vibrator != null

    /** Starts the ringtone and the vibration; a second call while ringing does nothing. */
    fun start() {
        if (ringing) {
            return
        }
        startRingtone()
        startVibration()
    }

    fun stop() {
        main.removeCallbacks(restart)
        ringtone?.stop()
        ringtone = null
        vibrator?.cancel()
        vibrator = null
    }

    private fun startRingtone() {
        val uri = RingtoneManager.getActualDefaultRingtoneUri(context, RingtoneManager.TYPE_RINGTONE)
        if (uri == null) {
            // The user chose no ringtone (None): their calls ring silently, and so does this one.
            return
        }
        val ringtone = RingtoneManager.getRingtone(context, uri)
        if (ringtone == null) {
            Log.w(TAG, "the default ringtone $uri could not be opened; the call rings by vibration alone")
            return
        }
        ringtone.audioAttributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE)
            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            ringtone.isLooping = true
        }
        this.ringtone = ringtone
        ringtone.play()
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.P) {
            // Before Android 9 a ringtone cannot be asked to loop and says nothing when it ends,
            // so it is watched and played again from its end.
            main.postDelayed(restart, RESTART_CHECK_MILLIS)
        }
    }

    private val restart = object : Runnable {
        override fun run() {
            val ringtone = ringtone ?: return
            if (!ringtone.isPlaying) {
                ringtone.play()
            }
            main.postDelayed(this, RESTART_CHECK_MILLIS)
        }
    }

    private fun startVibration() {
        val vibrator = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            context.getSystemService(VibratorManager::class.java).defaultVibrator
        } else {
            @Suppress("DEPRECATION")
            context.getSystemService(Vibrator::class.java)
        }
        if (!vibrator.hasVibrator()) {
            return
        }
        val effect = VibrationEffect.createWaveform(RING_VIBRATION, 0)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            vibrator.vibrate(effect, VibrationAttributes.createForUsage(VibrationAttributes.USAGE_RINGTONE))
        } else {
            @Suppress("DEPRECATION")
            vibrator.vibrate(
                effect,
                AudioAttributes.Builder()
                    .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE)
                    .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                    .build(),
            )
        }
        this.vibrator = vibrator
    }

    private companion object {
        const val TAG = "AspenCall"
        /** A second on, a second off, repeated: the pattern the phone's own calls vibrate in. */
        val RING_VIBRATION = longArrayOf(0, 1_000, 1_000)
        const val RESTART_CHECK_MILLIS = 250L
    }
}
