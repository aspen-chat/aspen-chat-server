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
 * OkHttp, peer connections over WebRTC, the session from the account kept for push, the audio
 * mode and microphone, and the system's side of each call (`telecom`: the call in Telecom, the
 * service that keeps the process alive, and `routes`, where it plays), which follows the
 * engine's snapshots. The engine's methods are the call's commands and `snapshots`
 * is what it reports; the interface that commands it, and what plays and draws its tracks, are
 * later pieces, so nothing holds one of these yet.
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
    val snapshots = CallSnapshots()
    val credentials = CallCredentials(context, origin)
    val audio = CallAudio(context, media.factory, tracks)
    val routes = CallRoutes(context)
    val engine: Engine = Engine.start(
        Platform(CallHttp(), sockets, peerConnections ?: media),
        Options(origin, credentials, loopbackPlainText),
        snapshots,
    )
    val telecom = CallTelecom(context, engine, snapshots, audio, routes)

    override fun close() {
        // Leaving the call first lets Telecom's call end as the user's own leaving.
        engine.close()
        telecom.close()
    }

    companion object {
        /** Every track on the device, however many deployments' engines run. */
        val tracks = CallTracks()
        private var shared: CallPeerConnections? = null

        /** One WebRTC factory for the process. */
        @Synchronized
        fun peerConnections(context: Context): CallPeerConnections =
            shared ?: CallPeerConnections(context.applicationContext, tracks).also { shared = it }
    }
}
