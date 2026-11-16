package com.aspen_chat.client.call

import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.util.Log
import androidx.core.telecom.CallControlResult
import androidx.core.telecom.CallControlScope
import androidx.core.telecom.CallEndpointCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

/**
 * Where a call's sound goes on this device: Telecom's endpoints for the call it holds, each
 * typed, so the interface shows one list on every phone and the Speaker button is a shortcut
 * between two kinds of entry. No endpoint is told apart by its name, which is the system's and
 * for display alone. Changing the route goes through Telecom too (`requestEndpointChange`),
 * never through `AudioManager` beside it, which would fight Telecom for the same device.
 *
 * `CallTelecom` attaches the control scope of each call it adds, and detaches it when the call
 * ends, when the list is empty again. A kind chosen (`select`) is kept for the call as `chosen`:
 * asked before Telecom lists the call's endpoints, as the app asks as a call starts, it is applied
 * once they arrive, and a kind asked later is applied at once; Telecom's own moves afterwards (a
 * headset connected) stand, and `clear` or the call's end forgets the choice. While the route is
 * the earpiece the proximity sensor turns the screen off at the ear
 * (`PROXIMITY_SCREEN_OFF_WAKE_LOCK`), as the phone app does. Listeners hear of every change on
 * the main thread.
 */
class CallRoutes(context: Context) {
    enum class Kind {
        EARPIECE,
        SPEAKER,
        BLUETOOTH,
        WIRED_HEADSET,
        /** Streamed to another device, or a kind this build does not know. */
        OTHER,
        ;

        companion object {
            fun of(endpoint: CallEndpointCompat): Kind = when (endpoint.type) {
                CallEndpointCompat.TYPE_EARPIECE -> EARPIECE
                CallEndpointCompat.TYPE_SPEAKER -> SPEAKER
                CallEndpointCompat.TYPE_BLUETOOTH -> BLUETOOTH
                CallEndpointCompat.TYPE_WIRED_HEADSET -> WIRED_HEADSET
                else -> OTHER
            }
        }
    }

    /** One place the call can play: `name` is the system's, for display alone. */
    data class Route(val kind: Kind, val name: CharSequence, val current: Boolean)

    fun interface Listener {
        fun changed(routes: List<Route>)
    }

    /** Stops the listener it was given for. */
    inner class Subscription internal constructor(private val listener: Listener) {
        fun cancel() {
            synchronized(lock) { listeners.remove(listener) }
        }
    }

    private val lock = Any()
    private val main = Handler(Looper.getMainLooper())
    private val audioManager = context.applicationContext.getSystemService(AudioManager::class.java)
    private val proximity: PowerManager.WakeLock? = context.applicationContext.getSystemService(PowerManager::class.java)
        .takeIf { it.isWakeLockLevelSupported(PowerManager.PROXIMITY_SCREEN_OFF_WAKE_LOCK) }
        ?.newWakeLock(PowerManager.PROXIMITY_SCREEN_OFF_WAKE_LOCK, "aspen:call")
        ?.also { it.setReferenceCounted(false) }
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private var control: CallControlScope? = null
    /** The collections of the call's endpoints, cancelled on `detach`. */
    private var following: List<Job> = emptyList()
    private var available: List<CallEndpointCompat> = emptyList()
    private var current: CallEndpointCompat? = null
    private val listeners = ArrayList<Listener>()
    /** The kind chosen for the call, and whether it still waits for Telecom's endpoints to be applied. */
    private var wanted: Kind? = null
    private var wantedPending = false

    /** Whether this device has an earpiece, which a tablet lacks, whether or not a call is on. */
    val hasEarpiece: Boolean
        get() = hasEarpiece(audioManager)

    /** The kind chosen for this call (`select`), until `clear` or the call's end. */
    val chosen: Kind?
        get() = synchronized(lock) { wanted }

    /** The routes of the call Telecom holds, in Telecom's order (headset, Bluetooth, speaker, earpiece); none outside a call. */
    val routes: List<Route>
        get() = synchronized(lock) { routesOf(available, current) }

    /** The kind the call plays through now, or `null` outside a call. */
    val currentKind: Kind?
        get() = synchronized(lock) { current?.let(Kind::of) }

    /**
     * Chooses `kind` for the call and asks Telecom to move the call to its first route. Before
     * Telecom has listed the call's endpoints the choice is kept and applied once it does
     * (`true`); with them listed, `false` when none is of that kind, and the choice is not kept.
     * Listeners hear the choice at once (`chosen`), and the move itself as the current route
     * changes; a move Telecom refuses is logged.
     */
    fun select(kind: Kind): Boolean {
        var request: Pair<CallControlScope?, CallEndpointCompat>? = null
        var kept = true
        update {
            if (available.isEmpty()) {
                wanted = kind
                wantedPending = true
            } else {
                val endpoint = available.firstOrNull { Kind.of(it) == kind }
                if (endpoint == null) {
                    kept = false
                } else {
                    wanted = kind
                    wantedPending = false
                    request = control to endpoint
                }
            }
        }
        if (!kept) {
            return false
        }
        request?.let { (control, endpoint) ->
            if (control != null) {
                move(control, endpoint, kind)
            }
        }
        return true
    }

    /** Forgets the choice; the call stays where Telecom has it. */
    fun clear() {
        update {
            wanted = null
            wantedPending = false
        }
    }

    private fun move(control: CallControlScope, endpoint: CallEndpointCompat, kind: Kind) {
        scope.launch {
            val result = control.requestEndpointChange(endpoint)
            if (result is CallControlResult.Error) {
                Log.w(TAG, "Telecom would not move the call to $kind: error ${result.errorCode}")
            }
        }
    }

    /** Applies a choice made before Telecom listed the endpoints, once, as they arrive. Under `lock`. */
    private fun applyWanted() {
        val kind = wanted
        val control = control
        if (!wantedPending || kind == null || control == null || available.isEmpty()) {
            return
        }
        wantedPending = false
        val endpoint = available.firstOrNull { Kind.of(it) == kind }
        if (endpoint == null) {
            wanted = null
            return
        }
        move(control, endpoint, kind)
    }

    /** Calls `listener` on the main thread with the routes now, and with every change, until cancelled. */
    fun listen(listener: Listener): Subscription {
        val now = synchronized(lock) {
            listeners.add(listener)
            routesOf(available, current)
        }
        main.post { listener.changed(now) }
        return Subscription(listener)
    }

    /**
     * Follows the endpoints of the call `control` holds, until `detach`. The collections run in a
     * scope of this component's, not the control scope's, which a collection left running would
     * keep open after the call is disconnected.
     */
    internal fun attach(control: CallControlScope) {
        val following = listOf(
            scope.launch {
                control.availableEndpoints.collect { endpoints ->
                    update {
                        available = endpoints
                        applyWanted()
                    }
                }
            },
            scope.launch { control.currentCallEndpoint.collect { endpoint -> update { current = endpoint } } },
        )
        synchronized(lock) {
            this.control = control
            this.following = following
        }
    }

    internal fun detach() {
        val following = synchronized(lock) { following.also { this.following = emptyList() } }
        for (job in following) {
            job.cancel()
        }
        update {
            control = null
            available = emptyList()
            current = null
            wanted = null
            wantedPending = false
        }
    }

    private fun update(change: () -> Unit) {
        val (routes, atEar, told) = synchronized(lock) {
            change()
            Triple(routesOf(available, current), current?.let(Kind::of) == Kind.EARPIECE, listeners.toList())
        }
        holdProximity(atEar)
        main.post {
            for (listener in told) {
                listener.changed(routes)
            }
        }
    }

    private fun holdProximity(atEar: Boolean) {
        val proximity = proximity ?: return
        if (atEar && !proximity.isHeld) {
            // For as long as the call plays at the ear; released when the route or the call changes.
            proximity.acquire()
        } else if (!atEar && proximity.isHeld) {
            proximity.release()
        }
    }

    private fun routesOf(available: List<CallEndpointCompat>, current: CallEndpointCompat?): List<Route> =
        available.sorted().map { Route(Kind.of(it), it.name, it.identifier == current?.identifier) }

    companion object {
        private const val TAG = "AspenCall"

        /** Whether the device has an earpiece, read without a call or an engine. */
        fun hasEarpiece(context: Context): Boolean = hasEarpiece(context.applicationContext.getSystemService(AudioManager::class.java))

        private fun hasEarpiece(audioManager: AudioManager): Boolean =
            audioManager.getDevices(AudioManager.GET_DEVICES_OUTPUTS).any { it.type == AudioDeviceInfo.TYPE_BUILTIN_EARPIECE }
    }
}
