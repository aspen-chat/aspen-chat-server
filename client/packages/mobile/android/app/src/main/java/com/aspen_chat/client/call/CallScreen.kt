package com.aspen_chat.client.call

import android.content.ComponentCallbacks
import android.content.Context
import android.content.Intent
import android.content.res.Configuration
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.projection.MediaProjection
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.DisplayMetrics
import android.util.Log
import android.view.WindowManager
import androidx.annotation.RequiresApi
import com.aspen_chat.client.voice.TrackId
import java.util.UUID
import org.webrtc.AudioSource
import org.webrtc.AudioTrack
import org.webrtc.MediaConstraints
import org.webrtc.ScreenCapturerAndroid
import org.webrtc.SurfaceTextureHelper
import org.webrtc.VideoSource
import org.webrtc.VideoTrack
import org.webrtc.audio.JavaAudioDeviceModule

/** Why the screen could not be shared, as the page's `ShareFailure` names it and the plugin rejects or its `share` event says. */
enum class CallScreenFailure(val code: String) {
    /** The user declined the system's consent. */
    DENIED("denied"),
    /** This device cannot capture its screen. */
    UNSUPPORTED("unsupported"),
    /** The capture would not start, or ended on its own. */
    FAILED("failed"),
}

/**
 * The call's share on this device: the screen captured by the media projection the user
 * consented to (`MediaProjectionManager.createScreenCaptureIntent`, whose answer `start` takes),
 * through WebRTC's `ScreenCapturerAndroid`, as a video track, and with its sound as an audio track
 * recording what other apps play (`AudioPlaybackCaptureConfiguration`, Android 10 and later; the
 * call's own sound, a voice communication, is left out), both of a WebRTC factory of the share's
 * own: a factory has one audio capture for every connection it makes, so the share's sound is the
 * capture of `CallPeerConnections.captureFactory`, made here for each share with an audio device
 * module whose `AudioRecord` is the playback capture (`JavaAudioDeviceModule.Builder
 * .setAudioRecordFactory`, which `scripts/webrtc_patches/android-audio-record-factory.patch`
 * gives WebRTC). The engine sends the tracks on the capture connection.
 *
 * The projection needs the call's service to run with the `mediaProjection` kind first
 * (`CallService.setProjection`), which `start` waits for; the capture is the display's own size,
 * no larger than `MAX_SIDE` on a side, followed through a turn (`changeCaptureFormat` on a
 * configuration change). The projection's own stop (the system's notification, or another
 * projection taking over) ends the share. Call on the main thread; callbacks arrive there.
 *
 * Stopping takes the capture and the projection down at once and retires the factory
 * (`CaptureFactory.retire`), which disposes itself, its module, and the share's tracks and
 * sources once the engine has closed the capture connection it made from it, on the engine's
 * thread: the engine is told to stop the sources beside the stop (`AspenCallPlugin`), and closes
 * the connection after replacing its tracks, so the factory must outlive that.
 */
class CallScreen(
    context: Context,
    private val media: CallPeerConnections,
    private val tracks: CallTracks,
) {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private var factory: CaptureFactory? = null
    /** The last share's factory, retired at its stop, for the proof to see go. */
    private var retired: CaptureFactory? = null
    private var helper: SurfaceTextureHelper? = null
    private var capturer: ScreenCapturerAndroid? = null
    private var videoSource: VideoSource? = null
    private var audioSource: AudioSource? = null
    private var turns: ComponentCallbacks? = null
    private var size: Pair<Int, Int>? = null
    var videoTrack: VideoTrack? = null
        private set
    var audioTrack: AudioTrack? = null
        private set
    /** The account whose engine the share is for, while one is asked for or on. */
    var account: CallAccount? = null
        private set
    private var onEnded: ((CallScreenFailure?) -> Unit)? = null

    /** Whether a share is asked for or on. */
    val isActive: Boolean
        get() = account != null

    /** Whether the share's media flows. */
    val isOn: Boolean
        get() = videoTrack != null

    /** How many frames the capture has taken, or -1 without one; for the proof's diagnosis. */
    val framesCaptured: Long
        get() = capturer?.numCapturedFrames ?: -1

    /**
     * Whether the last share's factory and module are gone, which they are once the engine has
     * closed its capture connection; true before any share. For the proof.
     */
    val factoryDisposed: Boolean
        get() = retired?.isDisposed ?: true

    /**
     * Starts capturing the screen the user consented to (`projection`, the consent's answer) for
     * `account`'s engine, with the sound of other apps when `audio` and the system can capture it;
     * `onStarted` is called with the video track and, with sound, the audio track once the
     * capture runs, which is when the engine is given them; `onEnded` when the share is over,
     * with why when it did not simply stop. A share already asked for is left as it is.
     */
    fun start(
        account: CallAccount,
        projection: Intent,
        audio: Boolean,
        onStarted: (TrackId, TrackId?) -> Unit,
        onEnded: (CallScreenFailure?) -> Unit,
    ) {
        if (this.account != null) {
            return
        }
        this.account = account
        this.onEnded = onEnded
        CallService.setProjection(context, true) { allowed ->
            main.post {
                if (this.account != account) {
                    return@post
                }
                if (!allowed) {
                    Log.w(TAG, "the call's service does not run with the media projection's kind, so no projection can be made")
                    end(CallScreenFailure.FAILED)
                    return@post
                }
                capture(projection, audio, onStarted)
            }
        }
    }

    private fun capture(projection: Intent, audio: Boolean, onStarted: (TrackId, TrackId?) -> Unit) {
        val capturer = ScreenCapturerAndroid(
            projection,
            object : MediaProjection.Callback() {
                override fun onStop() {
                    main.post { if (this@CallScreen.capturer === capturer) end(null) }
                }
            },
        )
        val withAudio = audio && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q
        val audioModule = JavaAudioDeviceModule.builder(context)
            .setUseHardwareAcousticEchoCanceler(false)
            .setUseHardwareNoiseSuppressor(false)
            .setUseStereoInput(true)
            .apply {
                if (withAudio) {
                    setAudioRecordFactory { sampleRate, channelConfig, audioFormat, bufferSizeInBytes ->
                        // The capturer makes its projection as it starts, before the module records.
                        val projection = capturer.mediaProjection
                            ?: throw IllegalStateException("the screen capture has no projection to record the playback of")
                        playbackCapture(projection, sampleRate, channelConfig, audioFormat, bufferSizeInBytes)
                    }
                }
            }
            .createAudioDeviceModule()
        val factory = media.makeCaptureFactory(audioModule)
        val helper = SurfaceTextureHelper.create("aspen-screen", media.eglContext)
        val videoSource = factory.factory.createVideoSource(true)
        val videoTrack = factory.factory.createVideoTrack("share-${UUID.randomUUID()}", videoSource)
        var audioTrack: AudioTrack? = null
        if (withAudio) {
            val source = factory.factory.createAudioSource(shareAudioConstraints())
            audioSource = source
            audioTrack = factory.factory.createAudioTrack("share-audio-${UUID.randomUUID()}", source)
        }
        this.factory = factory
        this.helper = helper
        this.capturer = capturer
        this.videoSource = videoSource
        this.videoTrack = videoTrack
        this.audioTrack = audioTrack
        media.captureFactory = factory
        tracks.add(videoTrack)
        audioTrack?.let { tracks.add(it) }
        capturer.initialize(helper, context, videoSource.capturerObserver)
        val (width, height) = displaySize()
        size = width to height
        // The projection and its virtual display are made here, so the capture runs once this
        // returns; frames follow as the screen changes. The engine is given the tracks now: the
        // source hands frames on only once a sender wants them, so a first frame cannot be the
        // signal to send.
        try {
            capturer.startCapture(width, height, FPS)
        } catch (e: RuntimeException) {
            Log.w(TAG, "the screen capture would not start", e)
            end(CallScreenFailure.FAILED)
            return
        }
        followTurns()
        onStarted(videoTrack.id(), audioTrack?.id())
    }

    /**
     * The `AudioRecord` the share's audio device module records from: what other apps play
     * (media, games, and sound of no stated usage), never a voice communication, which is the
     * call's own sound, at the format and buffer size the module asks for.
     */
    @RequiresApi(Build.VERSION_CODES.Q)
    private fun playbackCapture(
        projection: MediaProjection,
        sampleRate: Int,
        channelConfig: Int,
        audioFormat: Int,
        bufferSizeInBytes: Int,
    ): AudioRecord {
        val configuration = AudioPlaybackCaptureConfiguration.Builder(projection)
            .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
            .addMatchingUsage(AudioAttributes.USAGE_GAME)
            .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
            .build()
        return AudioRecord.Builder()
            .setAudioPlaybackCaptureConfig(configuration)
            .setAudioFormat(
                AudioFormat.Builder()
                    .setEncoding(audioFormat)
                    .setSampleRate(sampleRate)
                    .setChannelMask(channelConfig)
                    .build(),
            )
            .setBufferSizeInBytes(bufferSizeInBytes)
            .build()
    }

    /** The display's size in pixels, no larger than `MAX_SIDE` on a side, both even. */
    private fun displaySize(): Pair<Int, Int> {
        val manager = context.getSystemService(WindowManager::class.java)
        val (width, height) = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val bounds = manager.maximumWindowMetrics.bounds
            bounds.width() to bounds.height()
        } else {
            val metrics = DisplayMetrics()
            @Suppress("DEPRECATION")
            manager.defaultDisplay.getRealMetrics(metrics)
            metrics.widthPixels to metrics.heightPixels
        }
        val longest = maxOf(width, height)
        val ratio = if (longest > MAX_SIDE) MAX_SIDE.toDouble() / longest else 1.0
        return ((width * ratio).toInt() and 1.inv()) to ((height * ratio).toInt() and 1.inv())
    }

    /** Follows a turn of the device: the capture takes the display's new size. */
    private fun followTurns() {
        val callbacks = object : ComponentCallbacks {
            override fun onConfigurationChanged(newConfig: Configuration) {
                val capturer = capturer ?: return
                val (width, height) = displaySize()
                if (size != width to height) {
                    size = width to height
                    capturer.changeCaptureFormat(width, height, FPS)
                }
            }

            override fun onLowMemory() {}
        }
        context.registerComponentCallbacks(callbacks)
        turns = callbacks
    }

    /** Ends the share: the capture and the projection now, the factory once its connection has closed. */
    fun stop() {
        if (account != null) {
            Log.i(TAG, "the share is stopped")
        }
        release()
    }

    private fun end(failure: CallScreenFailure?) {
        Log.i(TAG, "the share ended on its own (${failure?.code ?: "stopped"})")
        val ended = onEnded
        release()
        ended?.invoke(failure)
    }

    private fun release() {
        turns?.let { context.unregisterComponentCallbacks(it) }
        turns = null
        val capturer = capturer
        this.capturer = null
        if (capturer != null) {
            try {
                capturer.stopCapture()
            } catch (interrupted: InterruptedException) {
                Thread.currentThread().interrupt()
            }
            capturer.dispose()
        }
        videoTrack?.let { tracks.remove(it.id()) }
        audioTrack?.let { tracks.remove(it.id()) }
        helper?.dispose()
        media.captureFactory = null
        factory?.let {
            it.retire(listOfNotNull(videoTrack, audioTrack), listOfNotNull(videoSource, audioSource))
            retired = it
        }
        videoTrack = null
        audioTrack = null
        videoSource = null
        audioSource = null
        helper = null
        factory = null
        size = null
        onEnded = null
        if (account != null) {
            account = null
            CallService.setProjection(context, false)
        }
    }

    companion object {
        private const val TAG = "AspenCall"
        /** The longest side the screen is captured at; the engine's encoding bounds what is sent. */
        const val MAX_SIDE = 1920
        const val FPS = 30

        /**
         * The constraints of the share's audio source: the sound is an application's, not a
         * voice in a room, so none of WebRTC's voice processing is applied over it.
         */
        fun shareAudioConstraints(): MediaConstraints = MediaConstraints().apply {
            for (name in listOf("googEchoCancellation", "googNoiseSuppression", "googAutoGainControl", "googHighpassFilter")) {
                mandatory.add(MediaConstraints.KeyValuePair(name, "false"))
            }
        }
    }
}
