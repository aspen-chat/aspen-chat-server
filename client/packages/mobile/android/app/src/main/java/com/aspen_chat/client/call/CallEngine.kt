package com.aspen_chat.client.call

import android.content.Context
import com.aspen_chat.client.voice.Engine
import com.aspen_chat.client.voice.Options
import com.aspen_chat.client.voice.PeerConnectionFactory
import com.aspen_chat.client.voice.Platform
import com.aspen_chat.client.voice.SocketFactory

/**
 * The call engine (`voice_client`, reached through its generated `com.aspen_chat.client.voice`
 * package) with this device's platform under it, for one deployment: HTTP and WebSockets over
 * OkHttp, peer connections over WebRTC, the files its transfers read and write (`CallFiles`, over
 * what the `AspenFiles` plugin's pickers chose), the session from the account kept for push, the
 * audio mode and microphone, the call's sounds (`sounds`, played as the engine cues them), and
 * the system's side of each call (`telecom`: the call in Telecom, the
 * service that keeps the process alive, and `routes`, where it plays), which follows the
 * engine's snapshots. The engine's methods are the call's commands and `snapshots` is what it
 * reports; `AspenCallPlugin` commands it for the web view, and the process's `video` draws its
 * video tracks and `camera` captures the camera it sends.
 *
 * Starts an engine for the deployment at `origin` (its `public_url`), which the device's account
 * there, kept by the web view on sign-in, is the session of. `loopbackPlainText` lets a
 * development deployment's voice servers be reached over plain HTTP at this device's own
 * addresses; a release build never allows it. A test stands between the engine and the platform
 * by giving `media` of its own (a `CallPeerConnections` on another audio device module, whose
 * factory the microphone then comes from too), `sockets` wrapping `CallSockets`, or
 * `peerConnections` wrapping `media`. Closing it leaves any call and stops the engine's thread,
 * waiting for it, so it is never closed from a snapshot listener.
 */
class CallEngine(
    context: Context,
    origin: String,
    loopbackPlainText: Boolean = false,
    media: CallPeerConnections = peerConnections(context),
    sockets: SocketFactory = CallSockets(),
    peerConnections: PeerConnectionFactory? = null,
) : AutoCloseable {
    val sounds = CallSounds(context)
    val snapshots = CallSnapshots(sounds)
    val credentials = CallCredentials(context, origin)
    val audio = CallAudio(context, media.factory, tracks)
    val routes = CallRoutes(context)
    val engine: Engine = Engine.start(
        Platform(CallHttp(), sockets, peerConnections ?: media, CallFiles(context)),
        Options(origin, credentials, loopbackPlainText),
        snapshots,
    )
    val telecom = CallTelecom(context, engine, snapshots, audio, routes)

    override fun close() {
        // Leaving the call first lets Telecom's call end as the user's own leaving.
        engine.close()
        telecom.close()
        sounds.release()
    }

    companion object {
        /** Every track on the device, however many deployments' engines run. */
        val tracks = CallTracks()
        /** Where the call's pictures are drawn, whichever deployment's engine the call is. */
        val video = CallVideo(tracks)
        private var shared: CallPeerConnections? = null
        private var sharedCamera: CallCamera? = null

        /** One WebRTC factory for the process. */
        @Synchronized
        fun peerConnections(context: Context): CallPeerConnections =
            shared ?: CallPeerConnections(context.applicationContext, tracks).also { shared = it }

        /** One camera for the process, on the factory's EGL context. */
        @Synchronized
        fun camera(context: Context): CallCamera = sharedCamera ?: run {
            val media = peerConnections(context)
            CallCamera(context.applicationContext, media.factory, media.eglContext, tracks).also { sharedCamera = it }
        }
    }
}
