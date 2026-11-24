package com.aspen_chat.client.call

import android.content.Context
import android.os.Handler
import android.os.Looper
import android.util.Log
import com.aspen_chat.client.voice.TrackId
import java.util.UUID
import org.webrtc.Camera2Enumerator
import org.webrtc.CameraVideoCapturer
import org.webrtc.EglBase
import org.webrtc.PeerConnectionFactory
import org.webrtc.SurfaceTextureHelper
import org.webrtc.VideoSource
import org.webrtc.VideoTrack

/**
 * A camera the phone has, as Camera2 lists it: its id, no name (Camera2 gives none; the page
 * names it by its facing), and which way it faces (`PhoneCamera` in the app's `nativeCall.ts`).
 */
data class CallCameraDevice(val id: String, val facing: Facing?) {
    enum class Facing(val wire: String) {
        FRONT("front"),
        BACK("back"),
    }
}

/** Why the camera could not be opened, as the page's `CameraFailure` names it and the plugin rejects with it. */
enum class CallCameraFailure(val code: String) {
    /** The system refused the camera. */
    DENIED("denied"),
    /** The phone has no camera, or not the one asked for. */
    NONE("none"),
    /** The camera would not start. */
    FAILED("failed"),
}

/**
 * The call's camera on this device: one Camera2 capturer and one track for the process, opened
 * through `start` as the engine's `camera` source and kept until `stop`, moved between cameras by
 * `start` again with the track unchanged (`switchCamera`), so the engine's sender needs nothing.
 * The frames go to the track's `VideoSource` through a `SurfaceTextureHelper` on the factory's
 * EGL context, which WebRTC adapts and encodes; the self preview is drawn from the same track
 * (`CallVideo`). `CAMERA` must be granted before `start` (the plugin asks). Call on the main
 * thread; completions arrive there. The format asked of the camera is the largest at most
 * `MAX_WIDTH` by `MAX_HEIGHT` at `FPS`, which bounds what the encoder is given; the engine's
 * encoding bounds what is sent.
 */
class CallCamera(
    context: Context,
    private val factory: PeerConnectionFactory,
    private val eglContext: EglBase.Context,
    private val tracks: CallTracks,
) {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private val enumerator = Camera2Enumerator(this.context)
    private var helper: SurfaceTextureHelper? = null
    private var capturer: CameraVideoCapturer? = null
    private var source: VideoSource? = null
    var track: VideoTrack? = null
        private set
    /** The camera open now. */
    var device: CallCameraDevice? = null
        private set
    /** The account whose engine was given the track (`startSource`), while the camera is on. */
    var account: CallAccount? = null
        private set

    /** The cameras the phone has, listed without asking the user, who is asked as one opens. */
    val devices: List<CallCameraDevice>
        get() = enumerator.deviceNames.map(::device)

    private fun device(name: String) = CallCameraDevice(
        name,
        when {
            enumerator.isFrontFacing(name) -> CallCameraDevice.Facing.FRONT
            enumerator.isBackFacing(name) -> CallCameraDevice.Facing.BACK
            else -> null
        },
    )

    val isOn: Boolean
        get() = track != null

    /**
     * Opens the camera `id` names (the phone's default, its front camera, for `null`) for
     * `account`'s engine, answering the track to send and the camera opened once its first frame
     * has arrived, or why none opened. A camera already on is moved instead, with no gap in the
     * frames but the camera's own.
     */
    fun start(id: String?, account: CallAccount, completion: (Result<Pair<TrackId, CallCameraDevice>>) -> Unit) {
        val names = enumerator.deviceNames.toList()
        val chosen = when {
            id != null -> names.firstOrNull { it == id }
            else -> names.firstOrNull(enumerator::isFrontFacing) ?: names.firstOrNull()
        }
        if (chosen == null) {
            completion(Result.failure(CameraException(CallCameraFailure.NONE)))
            return
        }
        this.account = account
        val running = capturer
        if (running != null) {
            running.switchCamera(
                object : CameraVideoCapturer.CameraSwitchHandler {
                    override fun onCameraSwitchDone(isFrontCamera: Boolean) {
                        main.post {
                            device = device(chosen)
                            val track = track
                            if (track == null) {
                                completion(Result.failure(CameraException(CallCameraFailure.FAILED)))
                            } else {
                                completion(Result.success(track.id() to device(chosen)))
                            }
                        }
                    }

                    override fun onCameraSwitchError(errorDescription: String) {
                        Log.w(TAG, "the camera would not move to $chosen: $errorDescription")
                        main.post { completion(Result.failure(CameraException(CallCameraFailure.FAILED))) }
                    }
                },
                chosen,
            )
            return
        }
        val helper = SurfaceTextureHelper.create("aspen-camera", eglContext)
        val source = factory.createVideoSource(false)
        val track = factory.createVideoTrack("camera-${UUID.randomUUID()}", source)
        var answered = false
        val events = object : CameraVideoCapturer.CameraEventsHandler {
            override fun onCameraError(errorDescription: String) {
                Log.w(TAG, "the camera failed: $errorDescription")
                main.post {
                    if (!answered) {
                        answered = true
                        stop()
                        completion(Result.failure(CameraException(CallCameraFailure.FAILED)))
                    } else {
                        stop()
                    }
                }
            }

            override fun onCameraDisconnected() {
                onCameraError("disconnected")
            }

            override fun onCameraFreezed(errorDescription: String) {
                Log.w(TAG, "the camera froze: $errorDescription")
            }

            override fun onCameraOpening(cameraName: String) {}

            override fun onFirstFrameAvailable() {
                main.post {
                    if (!answered) {
                        answered = true
                        device = device(chosen)
                        completion(Result.success(track.id() to device(chosen)))
                    }
                }
            }

            override fun onCameraClosed() {}
        }
        val capturer = enumerator.createCapturer(chosen, events)
        if (capturer == null) {
            helper.dispose()
            track.dispose()
            source.dispose()
            completion(Result.failure(CameraException(CallCameraFailure.FAILED)))
            return
        }
        this.helper = helper
        this.source = source
        this.track = track
        this.capturer = capturer
        tracks.add(track)
        capturer.initialize(helper, context, source.capturerObserver)
        capturer.startCapture(MAX_WIDTH, MAX_HEIGHT, FPS)
    }

    /** Stops the camera and lets its track go. */
    fun stop() {
        val capturer = capturer ?: return
        this.capturer = null
        try {
            capturer.stopCapture()
        } catch (interrupted: InterruptedException) {
            Thread.currentThread().interrupt()
        }
        capturer.dispose()
        track?.let { tracks.remove(it.id()) }
        track?.dispose()
        source?.dispose()
        helper?.dispose()
        track = null
        source = null
        helper = null
        device = null
        account = null
    }

    class CameraException(val failure: CallCameraFailure) : Exception(failure.code)

    companion object {
        private const val TAG = "AspenCall"
        const val MAX_WIDTH = 1280
        const val MAX_HEIGHT = 720
        const val FPS = 30
    }
}
