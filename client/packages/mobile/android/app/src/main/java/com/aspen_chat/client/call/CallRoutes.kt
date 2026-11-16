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
 * ends, when the list is empty again. While the route is the earpiece the proximity sensor turns
 * the screen off at the ear (`PROXIMITY_SCREEN_OFF_WAKE_LOCK`), as the phone app does.
 * Listeners hear of every change on the main thread.
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

    /** Whether this device has an earpiece, which a tablet lacks, whether or not a call is on. */
    val hasEarpiece: Boolean
        get() = audioManager.getDevices(AudioManager.GET_DEVICES_OUTPUTS).any { it.type == AudioDeviceInfo.TYPE_BUILTIN_EARPIECE }

    /** The routes of the call Telecom holds, in Telecom's order (headset, Bluetooth, speaker, earpiece); none outside a call. */
    val routes: List<Route>
        get() = synchronized(lock) { routesOf(available, current) }

    /** The kind the call plays through now, or `null` outside a call. */
    val currentKind: Kind?
        get() = synchronized(lock) { current?.let(Kind::of) }

    /**
     * Asks Telecom to move the call to the first route of `kind`. Answers `false` when no call is
     * on or no route of that kind is available; the move itself is announced to listeners as
     * the current route changes, or logged when Telecom refuses it.
     */
    fun select(kind: Kind): Boolean {
        val (control, endpoint) = synchronized(lock) {
            control to available.firstOrNull { Kind.of(it) == kind }
        }
        if (control == null || endpoint == null) {
            return false
        }
        scope.launch {
            val result = control.requestEndpointChange(endpoint)
            if (result is CallControlResult.Error) {
                Log.w(TAG, "Telecom would not move the call to $kind: error ${result.errorCode}")
            }
        }
        return true
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
            scope.launch { control.availableEndpoints.collect { endpoints -> update { available = endpoints } } },
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

    private companion object {
        const val TAG = "AspenCall"
    }
}
