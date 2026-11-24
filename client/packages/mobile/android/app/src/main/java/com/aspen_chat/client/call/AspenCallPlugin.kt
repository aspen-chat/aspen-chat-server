package com.aspen_chat.client.call

import android.Manifest
import android.app.PictureInPictureParams
import android.content.Context
import android.content.Intent
import android.graphics.Rect
import android.media.projection.MediaProjectionManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.util.Rational
import android.widget.FrameLayout
import androidx.activity.result.ActivityResult
import com.aspen_chat.client.CallPictureInPictureHost
import com.aspen_chat.client.R
import com.aspen_chat.client.voice.CallState
import com.aspen_chat.client.voice.CallStatus
import com.aspen_chat.client.voice.MediaSource
import com.aspen_chat.client.voice.SessionEndReason
import com.aspen_chat.client.voice.TransferMode
import com.aspen_chat.client.voice.WatchedVideo
import com.getcapacitor.JSArray
import com.getcapacitor.JSObject
import com.getcapacitor.PermissionState
import com.getcapacitor.Plugin
import com.getcapacitor.PluginCall
import com.getcapacitor.PluginMethod
import com.getcapacitor.annotation.ActivityCallback
import com.getcapacitor.annotation.CapacitorPlugin
import com.getcapacitor.annotation.Permission
import com.getcapacitor.annotation.PermissionCallback
import org.json.JSONObject

/**
 * The web view's way to the call engine (`AspenCall` in the app's `src/api/nativeCall.ts`, which
 * iOS's plugin of the same name answers too): the engine of each account the page names by its
 * origin and user id, from `CallEngines`, the process's one engine per account, which a DM's call
 * ringing the phone drives too, so a ring answered from the lock screen is the call the page then
 * shows; its snapshots are published to the page as `state` events and answered afresh by
 * `state`, each the JSON the engine writes of its state (`call_bridge_schema.json`), handed on
 * untouched beside the account's origin and user id, the
 * routes of the call Telecom holds, published as `routes` events, and the call's video: the
 * cameras (`CallCamera`), the holes the page reports for the pictures (`setTiles`, laid out by
 * `CallVideo` in the activity's `video_container` behind the web view, attached as the plugin
 * loads), each picture's shape back as a `shape` event, and the activity's picture-in-picture
 * mode as `pip` events (`CallPictureInPictureHost`, which `MainActivity` calls). The call's files
 * go to the engine by the references the `AspenFiles` plugin's pickers put in `FileRegistry`
 * (`offerFile`, `acceptOffer`), so no file's bytes cross the bridge. Joining asks for
 * `RECORD_AUDIO` first: granted, the microphone is opened and given to the engine; refused, the
 * engine is given none and fails the join itself where the channel lets the user speak
 * (`errorKind` `microphone`), as the web's call does when the browser refuses it. `startCamera`
 * asks for `CAMERA` the same way and rejects with the code `denied`, `none`, or `failed`
 * (`CallCameraFailure`) when none opens. `startScreenShare` asks for the screen through the
 * system's media projection dialog (`createScreenCaptureIntent`, the consent, which is the choice
 * of what to share) and resolves once the user has answered: consented, the capture starts
 * (`CallScreen`) and its tracks go to the engine as its external `screen` and `screenAudio` once
 * the first frame arrives, which the snapshots then show; declined, it rejects with the code
 * `denied` (`CallScreenFailure`); and a share that ends afterwards is a `share` event with why,
 * or none for the user's own stop. The system's side of each call (`CallTelecom`) follows the
 * engine's snapshots on its own. Everything touching an engine runs on the main thread, where
 * `CallTelecom` reads its state.
 *
 * `MainActivity` registers this plugin by its class name, since a build without the call layer
 * leaves this package out (`app/build.gradle`).
 */
@CapacitorPlugin(
    name = "AspenCall",
    permissions = [
        Permission(alias = AspenCallPlugin.MICROPHONE, strings = [Manifest.permission.RECORD_AUDIO]),
        Permission(alias = AspenCallPlugin.CAMERA, strings = [Manifest.permission.CAMERA]),
    ],
)
class AspenCallPlugin : Plugin(), CallPictureInPictureHost {
    /** The subscriptions that publish one deployment's engine's snapshots and routes and follow them. */
    private class Followed {
        var snapshots: CallSnapshots.Subscription? = null
        var routes: CallRoutes.Subscription? = null
        var camera: CallSnapshots.Subscription? = null
    }

    private val main = Handler(Looper.getMainLooper())
    /** The engines the page has named, each followed from its first naming. Main thread. */
    private val followed = HashMap<CallAccount, Followed>()
    private var shapes: CallVideo.Subscription? = null

    /**
     * The video's container is the activity's (`video_container`, behind the web view, which is
     * made transparent here so the page's holes show the pictures through it); the pictures the
     * page has reported are laid out in it from now.
     */
    override fun load() {
        val container = activity.findViewById<FrameLayout>(R.id.video_container)
        if (container != null) {
            bridge.webView.setBackgroundColor(android.graphics.Color.TRANSPARENT)
            CallEngine.video.attach(container, bridge.webView, CallEngine.peerConnections(context).eglContext)
        }
        shapes = CallEngine.video.listenShapes { shape -> notifyListeners("shape", shapeData(shape)) }
    }

    override fun handleOnDestroy() {
        shapes?.cancel()
        CallEngine.video.detach()
        super.handleOnDestroy()
    }

    // --- Engines ----------------------------------------------------------------------------------

    /** The account the call names, or `null` with the call rejected. */
    private fun accountOf(call: PluginCall): CallAccount? {
        val origin = call.getString("origin")
        val parsed = origin?.let(Uri::parse)
        if (origin == null || parsed == null || parsed.host.isNullOrEmpty() || parsed.scheme.isNullOrEmpty()) {
            call.reject("origin must be the deployment's address")
            return null
        }
        val userId = call.getString("userId")
        if (userId.isNullOrEmpty()) {
            call.reject("userId must name the account at the deployment")
            return null
        }
        return CallAccount(origin, userId)
    }

    /**
     * The account's engine (`CallEngines.forAccount`), followed for the page from the first time
     * the page names it: its snapshots go to the page as `state` events, its routes as
     * `routes` events, and the camera follows its call. Main thread.
     */
    private fun engineFor(account: CallAccount): CallEngine {
        val engine = CallEngines.forAccount(context, account)
        if (account !in followed) {
            val following = Followed()
            following.snapshots = engine.snapshots.listenJson { json -> notifyListeners("state", published(json, account)) }
            following.routes = engine.routes.listen { notifyListeners("routes", routesData()) }
            following.camera = engine.snapshots.listen { state ->
                cameraFollows(state, account)
                shareFollows(state, account)
            }
            followed[account] = following
        }
        return engine
    }

    /**
     * The camera is on only while its engine sends it: the call ending, or the voice server
     * refusing the camera, stops the capture, so the light goes off with the picture and the
     * service is no longer one that uses the camera.
     */
    private fun cameraFollows(state: CallState, account: CallAccount) {
        val camera = CallEngine.camera(context)
        if (!camera.isOn || camera.account != account) {
            return
        }
        val ended = state.status == CallStatus.IDLE || state.status == CallStatus.FAILED
        val refused = state.sourceRefused?.source == MediaSource.CAMERA
        if (ended || refused) {
            camera.stop()
            CallService.setCamera(context, false)
        }
    }

    /**
     * The share is on only while its engine sends it: the call ending, or the voice server
     * refusing the share, stops the capture, and the page hears why as a `share` event.
     */
    private fun shareFollows(state: CallState, account: CallAccount) {
        val screen = CallEngine.screen(context)
        if (!screen.isActive || screen.account != account) {
            return
        }
        val ended = state.status == CallStatus.IDLE || state.status == CallStatus.FAILED
        val refused = state.sourceRefused?.source == MediaSource.SCREEN || state.sourceRefused?.source == MediaSource.SCREEN_AUDIO
        if (ended || refused) {
            screen.stop()
            if (refused) {
                notifyListeners("share", shareData(account, null))
            }
        }
    }

    /** Runs `body` with the call's engine on the main thread, resolving the call after it. */
    private fun withEngine(call: PluginCall, body: (CallEngine) -> Unit) {
        val account = accountOf(call) ?: return
        main.post {
            body(engineFor(account))
            call.resolve()
        }
    }

    // --- Commands ---------------------------------------------------------------------------------

    /** Joins `channel`'s call as the account the call names, shown by the system as `name`, once the microphone is answered for. */
    @PluginMethod
    fun join(call: PluginCall) {
        if (accountOf(call) == null) {
            return
        }
        if (call.getString("channel").isNullOrEmpty()) {
            call.reject("channel is required")
            return
        }
        if (getPermissionState(MICROPHONE) == PermissionState.GRANTED) {
            joinWithMicrophone(call, granted = true)
        } else {
            requestPermissionForAlias(MICROPHONE, call, "microphoneAnswered")
        }
    }

    /** The system has answered for the microphone: the join goes on with or without it. */
    @PermissionCallback
    fun microphoneAnswered(call: PluginCall) {
        joinWithMicrophone(call, granted = getPermissionState(MICROPHONE) == PermissionState.GRANTED)
    }

    private fun joinWithMicrophone(call: PluginCall, granted: Boolean) {
        val channel = call.getString("channel") ?: return call.reject("channel is required")
        val name = call.getString("name")
        withEngine(call) { engine ->
            if (granted) {
                engine.engine.setMicrophone(engine.audio.openMicrophone())
            } else {
                engine.audio.closeMicrophone()
                engine.engine.setMicrophone(null)
            }
            engine.telecom.displayName = name ?: context.getString(R.string.app_name)
            engine.engine.join(channel)
        }
    }

    @PluginMethod
    fun leave(call: PluginCall) {
        withEngine(call) { engine -> engine.engine.leave() }
    }

    @PluginMethod
    fun setMuted(call: PluginCall) {
        val muted = call.getBoolean("muted") ?: return call.reject("muted is required")
        withEngine(call) { engine -> engine.engine.setMuted(muted) }
    }

    @PluginMethod
    fun setDeafened(call: PluginCall) {
        val deafened = call.getBoolean("deafened") ?: return call.reject("deafened is required")
        withEngine(call) { engine -> engine.engine.setDeafened(deafened) }
    }

    @PluginMethod
    fun setHeld(call: PluginCall) {
        val held = call.getBoolean("held") ?: return call.reject("held is required")
        withEngine(call) { engine -> engine.engine.setHeld(held) }
    }

    @PluginMethod
    fun setServerMuted(call: PluginCall) {
        val serverMuted = call.getBoolean("serverMuted") ?: return call.reject("serverMuted is required")
        withEngine(call) { engine -> engine.engine.setServerMuted(serverMuted) }
    }

    @PluginMethod
    fun setUserVolume(call: PluginCall) {
        val user = call.getString("user")
        if (user.isNullOrEmpty()) {
            return call.reject("user is required")
        }
        val source = audioSource(call.getString("source")) ?: return call.reject("source must be microphone or screenAudio")
        val gain = call.getDouble("gain")
        if (gain == null || !gain.isFinite() || gain < 0) {
            return call.reject("gain must be a number from 0")
        }
        withEngine(call) { engine -> engine.engine.setUserVolume(user, source, gain) }
    }

    @PluginMethod
    fun sessionEnded(call: PluginCall) {
        val session = call.getString("session")
        val channel = call.getString("channel")
        if (session.isNullOrEmpty() || channel.isNullOrEmpty()) {
            return call.reject("session and channel are required")
        }
        val reason = sessionEndReason(call.getString("reason")) ?: return call.reject("reason must be a voiceSessionEnded reason")
        withEngine(call) { engine -> engine.engine.sessionEnded(session, channel, reason) }
    }

    @PluginMethod
    fun acknowledgeEnd(call: PluginCall) {
        withEngine(call) { engine -> engine.engine.acknowledgeEnd() }
    }

    @PluginMethod
    fun clearStateRefusal(call: PluginCall) {
        withEngine(call) { engine -> engine.engine.clearStateRefusal() }
    }

    // --- Files ------------------------------------------------------------------------------------

    /**
     * Offers the file the `AspenFiles` plugin's picker chose, which `FileRegistry` holds by
     * `reference`, under the id the page chose; the engine opens it for its size, and one it
     * cannot open is the snapshot's `files.failure`.
     */
    @PluginMethod
    fun offerFile(call: PluginCall) {
        val offer = call.getString("offer")
        val reference = call.getString("reference")
        val name = call.getString("name")
        if (offer.isNullOrEmpty() || reference.isNullOrEmpty() || name.isNullOrEmpty()) {
            return call.reject("offer, reference, and name are required")
        }
        val allowDirect = call.getBoolean("allowDirect") ?: return call.reject("allowDirect is required")
        val validForSeconds = call.getInt("validForSeconds")
        if (validForSeconds == null || validForSeconds < 0) {
            return call.reject("validForSeconds must be a whole number from 0")
        }
        withEngine(call) { engine ->
            engine.engine.offerFile(offer, reference, name, allowDirect, validForSeconds.toUInt())
        }
    }

    @PluginMethod
    fun withdrawOffer(call: PluginCall) {
        val offer = call.getString("offer")
        if (offer.isNullOrEmpty()) {
            return call.reject("offer is required")
        }
        withEngine(call) { engine -> engine.engine.withdrawOffer(offer) }
    }

    /**
     * Accepts an offer into the place the `AspenFiles` plugin's save picker opened, which
     * `FileRegistry` holds by `destination`; the engine writes the file there as it arrives.
     */
    @PluginMethod
    fun acceptOffer(call: PluginCall) {
        val offer = call.getString("offer")
        if (offer.isNullOrEmpty()) {
            return call.reject("offer is required")
        }
        val mode = transferMode(call.getString("mode")) ?: return call.reject("mode must be directPreferred or relayOnly")
        val destination = call.getString("destination")
        if (destination.isNullOrEmpty()) {
            return call.reject("destination is required")
        }
        withEngine(call) { engine -> engine.engine.acceptOffer(offer, mode, destination) }
    }

    @PluginMethod
    fun cancelTransfer(call: PluginCall) {
        val (offer, peer) = transferOf(call) ?: return
        withEngine(call) { engine -> engine.engine.cancelTransfer(offer, peer) }
    }

    @PluginMethod
    fun dismissTransfer(call: PluginCall) {
        val (offer, peer) = transferOf(call) ?: return
        withEngine(call) { engine -> engine.engine.dismissTransfer(offer, peer) }
    }

    @PluginMethod
    fun clearFileFailure(call: PluginCall) {
        withEngine(call) { engine -> engine.engine.clearFileFailure() }
    }

    /** The transfer the call names, by its offer and its other side, or `null` with the call rejected. */
    private fun transferOf(call: PluginCall): Pair<String, String>? {
        val offer = call.getString("offer")
        val peer = call.getString("peer")
        if (offer.isNullOrEmpty() || peer.isNullOrEmpty()) {
            call.reject("offer and peer are required")
            return null
        }
        return Pair(offer, peer)
    }

    /** The two ways a receiver takes a file, as the page names them. */
    private fun transferMode(name: String?): TransferMode? = when (name) {
        "directPreferred" -> TransferMode.DIRECT_PREFERRED
        "relayOnly" -> TransferMode.RELAY_ONLY
        else -> null
    }

    // --- Video ------------------------------------------------------------------------------------

    /** The phone's cameras, each with its facing as the system says; the user is asked for the camera as one opens, not here. */
    @PluginMethod
    fun cameras(call: PluginCall) {
        main.post {
            val listed = JSArray()
            for (device in CallEngine.camera(context).devices) {
                listed.put(cameraData(device))
            }
            call.resolve(JSObject().put("cameras", listed))
        }
    }

    /**
     * Opens the camera `device` names (the phone's default for `null`) and gives the engine its
     * track to send, once `CAMERA` is answered for; resolves with the camera opened and its facing
     * (`OpenedCamera` in `nativeCall.ts`), or rejects with the code `denied`, `none`, or `failed`.
     */
    @PluginMethod
    fun startCamera(call: PluginCall) {
        if (accountOf(call) == null) {
            return
        }
        if (getPermissionState(CAMERA) == PermissionState.GRANTED) {
            openCamera(call)
        } else {
            requestPermissionForAlias(CAMERA, call, "cameraAnswered")
        }
    }

    /** The system has answered for the camera. */
    @PermissionCallback
    fun cameraAnswered(call: PluginCall) {
        if (getPermissionState(CAMERA) == PermissionState.GRANTED) {
            openCamera(call)
        } else {
            call.reject("the camera is not allowed", CallCameraFailure.DENIED.code)
        }
    }

    private fun openCamera(call: PluginCall) {
        val account = accountOf(call) ?: return
        val device = call.getString("device")
        main.post {
            val engine = engineFor(account)
            CallEngine.camera(context).start(device, account) { result ->
                result.fold(
                    onSuccess = { (track, opened) ->
                        engine.engine.startSource(MediaSource.CAMERA, track)
                        CallService.setCamera(context, true)
                        call.resolve(openedData(opened))
                    },
                    onFailure = { failure -> rejectCamera(call, failure) },
                )
            }
        }
    }

    /** Moves a camera that is on to the one `device` names, the track and the engine's sender unchanged. */
    @PluginMethod
    fun switchCamera(call: PluginCall) {
        val account = accountOf(call) ?: return
        val device = call.getString("device")
        main.post {
            val camera = CallEngine.camera(context)
            if (!camera.isOn) {
                return@post call.reject("the camera is off", CallCameraFailure.FAILED.code)
            }
            camera.start(device, account) { result ->
                result.fold(
                    onSuccess = { (_, opened) -> call.resolve(openedData(opened)) },
                    onFailure = { failure -> rejectCamera(call, failure) },
                )
            }
        }
    }

    private fun rejectCamera(call: PluginCall, failure: Throwable) {
        val code = (failure as? CallCamera.CameraException)?.failure ?: CallCameraFailure.FAILED
        val text = when (code) {
            CallCameraFailure.DENIED -> "the camera is not allowed"
            CallCameraFailure.NONE -> "the phone has no such camera"
            CallCameraFailure.FAILED -> "the camera would not start"
        }
        call.reject(text, code.code)
    }

    /** Stops sending the camera and turns it off. */
    @PluginMethod
    fun stopCamera(call: PluginCall) {
        withEngine(call) { engine ->
            engine.engine.stopSource(MediaSource.CAMERA)
            CallEngine.camera(context).stop()
            CallService.setCamera(context, false)
        }
    }

    /** Dismisses the voice server's refusal of a source. */
    @PluginMethod
    fun clearCameraError(call: PluginCall) {
        withEngine(call) { engine -> engine.engine.clearSourceRefusal() }
    }

    // --- Sharing ----------------------------------------------------------------------------------

    /**
     * Asks the system for the screen through its media projection dialog, and once consented
     * starts the capture (`CallScreen`), with other apps' sound when `audio`; resolves on the
     * consent, rejects with `denied` when it is declined. A share already asked for is left as it
     * is.
     */
    @PluginMethod
    fun startScreenShare(call: PluginCall) {
        if (accountOf(call) == null) {
            return
        }
        val screen = CallEngine.screen(context)
        if (screen.isActive) {
            return call.resolve()
        }
        val manager = context.getSystemService(Context.MEDIA_PROJECTION_SERVICE) as? MediaProjectionManager
            ?: return call.reject("the device cannot capture its screen", CallScreenFailure.UNSUPPORTED.code)
        startActivityForResult(call, manager.createScreenCaptureIntent(), "projectionAnswered")
    }

    /** The user has answered the media projection dialog. */
    @ActivityCallback
    fun projectionAnswered(call: PluginCall, result: ActivityResult) {
        val account = accountOf(call) ?: return
        val data = result.data
        if (result.resultCode != android.app.Activity.RESULT_OK || data == null) {
            return call.reject("sharing the screen is not allowed", CallScreenFailure.DENIED.code)
        }
        val audio = call.getBoolean("audio") ?: true
        main.post {
            val engine = engineFor(account)
            CallEngine.screen(context).start(
                account, data, audio,
                onStarted = { video, sound ->
                    engine.engine.startExternalSource(MediaSource.SCREEN, video)
                    if (sound != null) {
                        engine.engine.startExternalSource(MediaSource.SCREEN_AUDIO, sound)
                    }
                },
                onEnded = { failure ->
                    engine.engine.stopSource(MediaSource.SCREEN)
                    engine.engine.stopSource(MediaSource.SCREEN_AUDIO)
                    notifyListeners("share", shareData(account, failure))
                },
            )
            call.resolve()
        }
    }

    /** Ends the share, asked for or on. */
    @PluginMethod
    fun stopScreenShare(call: PluginCall) {
        withEngine(call) { engine ->
            engine.engine.stopSource(MediaSource.SCREEN)
            engine.engine.stopSource(MediaSource.SCREEN_AUDIO)
            CallEngine.screen(context).stop()
        }
    }

    /** Dismisses the voice server's refusal of a source. */
    @PluginMethod
    fun clearShareError(call: PluginCall) {
        withEngine(call) { engine -> engine.engine.clearSourceRefusal() }
    }

    /** Asks the engine to watch `video`, each a user's screen or camera. */
    @PluginMethod
    fun setWatched(call: PluginCall) {
        val listed = call.getArray("video") ?: return call.reject("video is required")
        val video = ArrayList<WatchedVideo>()
        for (i in 0 until listed.length()) {
            val entry = listed.optJSONObject(i)
            val user = entry?.optString("user")
            val source = videoSource(entry?.optString("source"))
            if (entry == null || user.isNullOrEmpty() || source == null) {
                return call.reject("each video names a user and a source, screen or camera")
            }
            video.add(WatchedVideo(user, source))
        }
        withEngine(call) { engine -> engine.engine.setWatched(video) }
    }

    /** Where the page's holes are (`TileReport` in `nativeCall.ts`), in its CSS pixels: the pictures are laid out there. */
    @PluginMethod
    fun setTiles(call: PluginCall) {
        val listed = call.getArray("tiles") ?: return call.reject("tiles is required")
        val tiles = ArrayList<CallVideoTile>()
        for (i in 0 until listed.length()) {
            val tile = listed.optJSONObject(i)?.let(::tileOf)
                ?: return call.reject("each tile names its tile, surface, rect, and how it is drawn")
            tiles.add(tile)
        }
        main.post {
            CallEngine.video.apply(tiles)
            updatePictureInPictureParams()
            call.resolve()
        }
    }

    // --- Picture in picture -------------------------------------------------------------------------

    /**
     * The activity enters the system's picture-in-picture mode on its own from Android 12 when the
     * user leaves while a picture is marked for it (`setAutoEnterEnabled`), and is told to on
     * earlier versions as the user leaves (`onUserLeaveHint`); the picture's shape sets the
     * window's. The page then shows that one picture alone, filling the shrunken page.
     */
    private fun updatePictureInPictureParams() {
        val pictured = CallEngine.video.pictured
        val params = pictureInPictureParams(pictured)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            params.setAutoEnterEnabled(pictured != null)
        }
        activity.setPictureInPictureParams(params.build())
    }

    private fun pictureInPictureParams(pictured: CallVideoTile?): PictureInPictureParams.Builder {
        val params = PictureInPictureParams.Builder()
        val shape = pictured?.let { CallEngine.video.shape(it.tile) }
        if (shape != null && shape.width > 0 && shape.height > 0) {
            // The window's shape is the picture's, within what the system allows (1:2.39 to 2.39:1).
            val ratio = (shape.width.toDouble() / shape.height).coerceIn(1.0 / 2.39, 2.39)
            params.setAspectRatio(Rational((ratio * 1000).toInt(), 1000))
            CallEngine.video.bounds(pictured.tile)?.let { params.setSourceRectHint(Rect(it)) }
        }
        return params
    }

    override fun onUserLeaveHint() {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.S && CallEngine.video.pictured != null) {
            activity.enterPictureInPictureMode(pictureInPictureParams(CallEngine.video.pictured).build())
        }
    }

    override fun onPictureInPictureModeChanged(inPictureInPictureMode: Boolean) {
        notifyListeners("pip", JSObject().put("active", inPictureInPictureMode))
    }

    /** Enters the picture-in-picture mode now, for a proof; what the user does is leave the app. */
    fun enterPictureInPicture(): Boolean {
        val pictured = CallEngine.video.pictured ?: return false
        return activity.enterPictureInPictureMode(pictureInPictureParams(pictured).build())
    }

    // --- State ------------------------------------------------------------------------------------

    /** The engine's latest snapshot, for a page that slept through the events. */
    @PluginMethod
    fun state(call: PluginCall) {
        val account = accountOf(call) ?: return
        main.post {
            val engine = engineFor(account)
            call.resolve(published(engine.snapshots.latestJson ?: engine.engine.stateJson(), account))
        }
    }

    // --- Routes -----------------------------------------------------------------------------------

    /** What the call can play through now: Telecom's endpoints for the call it holds, if any. */
    @PluginMethod
    fun routes(call: PluginCall) {
        main.post { call.resolve(routesData()) }
    }

    /**
     * Plays the call through the first route of `kind`, held for the call: asked of every
     * deployment's engine the process has made, since the one in a call is the one Telecom lists
     * routes for, and an engine not yet in one keeps the wish for when it is.
     */
    @PluginMethod
    fun setRoute(call: PluginCall) {
        val kind = routeKind(call.getString("kind")) ?: return call.reject("kind must be a route kind")
        main.post {
            val engines = CallEngines.all()
            val listed = engines.any { it.routes.routes.isNotEmpty() }
            val taken = engines.map { it.routes.select(kind) }
            if (listed && taken.none { it }) {
                call.reject("the device has no route of the kind ${call.getString("kind")}")
            } else {
                call.resolve()
            }
        }
    }

    /** Lets the choice go: the system's own route follows. */
    @PluginMethod
    fun clearRoute(call: PluginCall) {
        main.post {
            for (engine in CallEngines.all()) {
                engine.routes.clear()
            }
            call.resolve()
        }
    }

    /**
     * `PhoneRoutes` as the page reads it: the routes of the one call Telecom holds (an engine in a
     * call lists them; outside a call the list is empty), whether the device has an earpiece,
     * and the kind chosen. Main thread.
     */
    private fun routesData(): JSObject {
        val engines = CallEngines.all()
        val inCall = engines.map { it.routes }.firstOrNull { it.routes.isNotEmpty() }
        val routes = JSArray()
        for (route in inCall?.routes ?: emptyList()) {
            routes.put(
                JSObject()
                    .put("kind", routeKindName(route.kind))
                    .put("name", route.name.toString())
                    .put("inUse", route.current),
            )
        }
        val chosen = (inCall ?: engines.firstOrNull()?.routes)?.chosen
        return JSObject()
            .put("routes", routes)
            .put("hasReceiver", CallRoutes.hasEarpiece(context))
            .put("chosen", chosen?.let(::routeKindName) ?: JSONObject.NULL)
    }

    companion object {
        const val MICROPHONE = "microphone"
        const val CAMERA = "camera"

        /** The two pictures a participant sends, as the page names them. */
        fun videoSource(name: String?): MediaSource? = when (name) {
            "screen" -> MediaSource.SCREEN
            "camera" -> MediaSource.CAMERA
            else -> null
        }

        /** A hole as the page reports it (`TileReport` in `nativeCall.ts`), or `null` where a field is missing or malformed. */
        fun tileOf(entry: JSONObject): CallVideoTile? {
            val rect = entry.optJSONObject("rect") ?: return null
            val tile = entry.optString("tile")
            val surface = entry.optString("surface")
            if (tile.isEmpty() || surface.isEmpty()) {
                return null
            }
            val numbers = listOf("x", "y", "width", "height").map { rect.optDouble(it, Double.NaN) }
            if (numbers.any { !it.isFinite() } || numbers[2] < 0 || numbers[3] < 0) {
                return null
            }
            for (flag in listOf("visible", "mirrored", "cover", "fullScreen", "pictureInPicture")) {
                if (!entry.has(flag) || entry.opt(flag) !is Boolean) {
                    return null
                }
            }
            return CallVideoTile(
                tile, surface, numbers[0], numbers[1], numbers[2], numbers[3],
                visible = entry.getBoolean("visible"),
                mirrored = entry.getBoolean("mirrored"),
                cover = entry.getBoolean("cover"),
                fullScreen = entry.getBoolean("fullScreen"),
                pictureInPicture = entry.getBoolean("pictureInPicture"),
            )
        }

        /** `CallCameraDevice` as the page reads it (`PhoneCamera` in `nativeCall.ts`); Camera2 names no camera. */
        fun cameraData(device: CallCameraDevice): JSObject = JSObject()
            .put("id", device.id)
            .put("label", JSONObject.NULL)
            .put("facing", device.facing?.wire ?: JSONObject.NULL)

        /** The camera opened as the page reads it (`OpenedCamera` in `nativeCall.ts`). */
        fun openedData(device: CallCameraDevice): JSObject = JSObject()
            .put("device", device.id)
            .put("facing", device.facing?.wire ?: JSONObject.NULL)

        /** A share's end as the page reads it (`ShareEnded` in `nativeCall.ts`). */
        fun shareData(account: CallAccount, failure: CallScreenFailure?): JSObject = JSObject()
            .put("origin", account.origin)
            .put("userId", account.userId)
            .put("failure", failure?.code ?: JSONObject.NULL)

        /** `CallVideoShape` as the page reads it (`PictureShape` in `nativeCall.ts`). */
        fun shapeData(shape: CallVideoShape): JSObject = JSObject()
            .put("surface", shape.surface)
            .put("width", shape.width)
            .put("height", shape.height)
            .put("rotation", shape.rotation)

        /** The two sounds a participant sends whose volumes are set apart, as the page names them. */
        fun audioSource(name: String?): MediaSource? = when (name) {
            "microphone" -> MediaSource.MICROPHONE
            "screenAudio" -> MediaSource.SCREEN_AUDIO
            else -> null
        }

        /** `VoiceSessionEndReason` as the API server's event names it. */
        fun sessionEndReason(name: String?): SessionEndReason? = when (name) {
            "empty" -> SessionEndReason.EMPTY
            "serverLost" -> SessionEndReason.SERVER_LOST
            "idle" -> SessionEndReason.IDLE
            "serverRemoved" -> SessionEndReason.SERVER_REMOVED
            else -> null
        }

        /**
         * The page's `RouteKind` as Telecom's: a car plays over Bluetooth here and is listed as
         * such, so `car` names no route of its own, nor does `other`, which is what a kind this
         * build does not know is shown as and never asked for.
         */
        fun routeKind(name: String?): CallRoutes.Kind? = when (name) {
            "receiver" -> CallRoutes.Kind.EARPIECE
            "loudspeaker" -> CallRoutes.Kind.SPEAKER
            "bluetooth" -> CallRoutes.Kind.BLUETOOTH
            "wiredHeadset" -> CallRoutes.Kind.WIRED_HEADSET
            "car", "other" -> null
            else -> null
        }

        fun routeKindName(kind: CallRoutes.Kind): String = when (kind) {
            CallRoutes.Kind.EARPIECE -> "receiver"
            CallRoutes.Kind.SPEAKER -> "loudspeaker"
            CallRoutes.Kind.BLUETOOTH -> "bluetooth"
            CallRoutes.Kind.WIRED_HEADSET -> "wiredHeadset"
            CallRoutes.Kind.OTHER -> "other"
        }

        /**
         * A snapshot as the page reads it (`PublishedState` in `nativeCall.ts`): the engine's JSON
         * as written, and whose it is.
         */
        fun published(json: String, account: CallAccount): JSObject =
            JSObject().put("origin", account.origin).put("userId", account.userId).put("state", json)
    }
}
