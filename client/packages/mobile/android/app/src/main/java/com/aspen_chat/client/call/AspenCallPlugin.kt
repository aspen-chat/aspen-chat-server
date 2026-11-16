package com.aspen_chat.client.call

import android.Manifest
import android.content.pm.ApplicationInfo
import android.net.Uri
import android.os.Handler
import android.os.Looper
import com.aspen_chat.client.R
import com.aspen_chat.client.voice.MediaSource
import com.aspen_chat.client.voice.SessionEndReason
import com.getcapacitor.JSArray
import com.getcapacitor.JSObject
import com.getcapacitor.PermissionState
import com.getcapacitor.Plugin
import com.getcapacitor.PluginCall
import com.getcapacitor.PluginMethod
import com.getcapacitor.annotation.CapacitorPlugin
import com.getcapacitor.annotation.Permission
import com.getcapacitor.annotation.PermissionCallback
import org.json.JSONObject

/**
 * The web view's way to the call engine (`AspenCall` in the app's `src/api/nativeCall.ts`, which
 * iOS's plugin of the same name answers too): a `CallEngine` for each deployment the page names by
 * its origin, made on first use and kept for the process, whose snapshots are published to the
 * page as `state` events and answered afresh by `state`, each the JSON the engine writes of its
 * state (`call_bridge_schema.json`), handed on untouched beside the deployment's origin, and the
 * routes of the call Telecom holds, published as `routes` events. Joining asks for `RECORD_AUDIO` first: granted, the microphone is
 * opened and given to the engine; refused, the engine is given none and fails the join itself
 * where the channel lets the user speak (`errorKind` `microphone`), as the web's call does when
 * the browser refuses it. The system's side of each call (`CallTelecom`) follows the engine's
 * snapshots on its own. Everything touching an engine runs on the main thread, where `CallTelecom`
 * reads its state.
 *
 * `MainActivity` registers this plugin by its class name, since a build without the call layer
 * leaves this package out (`app/build.gradle`).
 */
@CapacitorPlugin(
    name = "AspenCall",
    permissions = [Permission(alias = AspenCallPlugin.MICROPHONE, strings = [Manifest.permission.RECORD_AUDIO])],
)
class AspenCallPlugin : Plugin() {
    /** One deployment's engine and the subscriptions that publish its snapshots and routes. */
    private class Held(val engine: CallEngine) {
        var snapshots: CallSnapshots.Subscription? = null
        var routes: CallRoutes.Subscription? = null
    }

    private val main = Handler(Looper.getMainLooper())
    /** Engines by the deployment's origin as `CallCredentials` keys accounts, without a trailing slash. Main thread. */
    private val engines = HashMap<String, Held>()

    // --- Engines ----------------------------------------------------------------------------------

    /** The deployment the call names, or `null` with the call rejected. */
    private fun originOf(call: PluginCall): String? {
        val origin = call.getString("origin")
        val parsed = origin?.let(Uri::parse)
        if (origin == null || parsed == null || parsed.host.isNullOrEmpty() || parsed.scheme.isNullOrEmpty()) {
            call.reject("origin must be the deployment's address")
            return null
        }
        return origin.trimEnd('/')
    }

    /**
     * The deployment's engine, made on first use. Main thread. A debuggable build lets the engine
     * reach a development deployment's voice servers over plain HTTP at this device's own
     * addresses, as the web client's development build does; a release build never does.
     */
    private fun engineAt(origin: String): CallEngine {
        engines[origin]?.let { return it.engine }
        val debuggable = context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE != 0
        val made = CallEngine(context, origin, loopbackPlainText = debuggable)
        val held = Held(made)
        held.snapshots = made.snapshots.listenJson { json -> notifyListeners("state", published(json, origin)) }
        held.routes = made.routes.listen { notifyListeners("routes", routesData()) }
        engines[origin] = held
        return made
    }

    /** Runs `body` with the call's engine on the main thread, resolving the call after it. */
    private fun withEngine(call: PluginCall, body: (CallEngine) -> Unit) {
        val origin = originOf(call) ?: return
        main.post {
            body(engineAt(origin))
            call.resolve()
        }
    }

    // --- Commands ---------------------------------------------------------------------------------

    /** Joins `channel`'s call at `origin`, shown by the system as `name`, once the microphone is answered for. */
    @PluginMethod
    fun join(call: PluginCall) {
        if (originOf(call) == null) {
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

    // --- State ------------------------------------------------------------------------------------

    /** The engine's latest snapshot, for a page that slept through the events. */
    @PluginMethod
    fun state(call: PluginCall) {
        val origin = originOf(call) ?: return
        main.post {
            val engine = engineAt(origin)
            call.resolve(published(engine.snapshots.latestJson ?: engine.engine.stateJson(), origin))
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
     * deployment's engine, since the one in a call is the one Telecom lists routes for, and an
     * engine not yet in one keeps the wish for when it is.
     */
    @PluginMethod
    fun setRoute(call: PluginCall) {
        val kind = routeKind(call.getString("kind")) ?: return call.reject("kind must be a route kind")
        main.post {
            val listed = engines.values.any { it.engine.routes.routes.isNotEmpty() }
            val taken = engines.values.map { it.engine.routes.select(kind) }
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
            for (held in engines.values) {
                held.engine.routes.clear()
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
        val inCall = engines.values.map { it.engine.routes }.firstOrNull { it.routes.isNotEmpty() }
        val routes = JSArray()
        for (route in inCall?.routes ?: emptyList()) {
            routes.put(
                JSObject()
                    .put("kind", routeKindName(route.kind))
                    .put("name", route.name.toString())
                    .put("inUse", route.current),
            )
        }
        val chosen = (inCall ?: engines.values.firstOrNull()?.engine?.routes)?.chosen
        return JSObject()
            .put("routes", routes)
            .put("hasReceiver", CallRoutes.hasEarpiece(context))
            .put("chosen", chosen?.let(::routeKindName) ?: JSONObject.NULL)
    }

    companion object {
        const val MICROPHONE = "microphone"

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
        fun published(json: String, origin: String): JSObject = JSObject().put("origin", origin).put("state", json)
    }
}
