package com.aspen_chat.client.call

import android.content.Context
import android.net.Uri
import android.os.ParcelUuid
import android.telecom.DisconnectCause
import android.util.Log
import androidx.core.telecom.CallAttributesCompat
import androidx.core.telecom.CallControlResult
import androidx.core.telecom.CallControlScope
import androidx.core.telecom.CallException
import androidx.core.telecom.CallsManager
import com.aspen_chat.client.R
import com.aspen_chat.client.voice.CallState
import com.aspen_chat.client.voice.CallStatus
import com.aspen_chat.client.voice.Engine
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The call engine's call as the system's: an outgoing call in Telecom for as long as the engine
 * is in one, so the system knows a call is on (other calls hold it, a watch or a car can end it,
 * the microphone stays the app's behind other apps, the audio focus is held for it), with
 * `CallService` keeping the process alive and showing the call, the audio mode set for it
 * (`CallAudio`), and its routes read from Telecom (`CallRoutes`).
 *
 * It follows the engine's snapshots rather than being commanded: whoever joins through the engine
 * (the interface, later a ring answered) has the system call made for them, and whatever ends the
 * engine's call (the user, a moderator, a failure) ends it. On the way in: the mode is set, the
 * service started with the call's notification, and the call added to Telecom, which is told the
 * call is active once the engine connects. On the way out: Telecom's call is disconnected with the
 * cause the engine's end says; once Telecom lets it go, the routes are detached, the service
 * stopped, and the mode given back. What Telecom asks goes to the engine: its disconnect leaves,
 * its hold (`onSetInactive`, a phone call taken, or an interruption Telecom's own audio focus
 * tells it of) mutes and deafens the user until it sets the call active again, and a mute from
 * the system is the engine's mute.
 *
 * The incoming direction (a DM call ringing this phone, reported from a push before the engine
 * joins) is a later piece; `attributes` is where it will differ.
 */
class CallTelecom(
    context: Context,
    private val engine: Engine,
    private val snapshots: CallSnapshots,
    private val audio: CallAudio,
    private val routes: CallRoutes,
) : AutoCloseable {
    /** Where the system's call stands; `IDLE` while the engine is in none. */
    enum class Phase { IDLE, DIALING, ACTIVE, HELD }

    private val context = context.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var call: SystemCall? = null
    private var closed = false
    private val subscription = snapshots.listen(::stateChanged)

    /**
     * What Telecom and the notification show the call as: the channel's or the other person's
     * name, which the interface that commands the engine sets before joining. The engine's
     * snapshots name the channel by id alone, so until it is set the call is shown as the app.
     */
    var displayName: CharSequence = context.getString(R.string.app_name)

    val phase: Phase
        get() = call?.phase ?: Phase.IDLE

    /** Telecom's id for the call, once it has added it; what `dumpsys telecom` lists it by. */
    val callId: ParcelUuid?
        get() = call?.control?.getCallId()

    /**
     * One call in Telecom, from `addCall` until Telecom has let it go. Read and written on the
     * main thread.
     */
    private inner class SystemCall(var video: Boolean) {
        var phase = Phase.DIALING
        var control: CallControlScope? = null
        /** The collection of Telecom's mute state, for the call's length. */
        var following: Job? = null
        var finished = false
        /** The engine's revision when the call's end was asked, which a join after it is past. */
        var endedAtRevision: ULong? = null
        /** Whether the engine has connected, so Telecom is to hold the call as active. */
        var activeAsked = false
        /** The cause the engine's end gave before Telecom handed the control scope over. */
        var endWanted: DisconnectCause? = null
        var disconnectedByTelecom = false
        var refused = false
        /** The user's own mute and deafen before the call was held, put back when it resumes. */
        var beforeHold: Pair<Boolean, Boolean>? = null
    }

    private fun stateChanged(state: CallState) {
        val call = call
        when {
            closed -> {}
            call == null && state.status.inCall -> begin(state)
            call == null -> {}
            state.status == CallStatus.CONNECTED -> connected(call, state)
            state.status == CallStatus.IDLE || state.status == CallStatus.FAILED -> end(call, state, causeOf(state))
        }
    }

    private fun begin(state: CallState) {
        val video = state.grants.camera || state.grants.shareScreen
        val call = SystemCall(video)
        this.call = call
        audio.configureForCall()
        CallService.start(context, displayName, video) { engine.leave() }
        val attributes = attributes(state, video)
        scope.launch {
            try {
                callsManager(context).addCall(
                    attributes,
                    onAnswer = {
                        // Only an incoming call is answered; Telecom never asks it of an outgoing one.
                    },
                    onDisconnect = { onMain { disconnectedByTelecom(call) } },
                    onSetActive = { onMain { held(call, held = false) } },
                    onSetInactive = { onMain { held(call, held = true) } },
                ) {
                    // Nothing is launched in the control scope itself: a collection left running
                    // in it would keep Telecom's session open after the call is disconnected.
                    val control = this
                    scope.launch { controlGiven(call, control) }
                }
            } catch (refused: CallException) {
                // A call Telecom will not take (another is being set up; too many) cannot be
                // the engine's either: the microphone would be the system's to take away.
                Log.w(TAG, "Telecom refused the call (error ${refused.code}); leaving it", refused)
                call.refused = true
                engine.leave()
            } finally {
                finished(call)
            }
        }
    }

    private fun attributes(state: CallState, video: Boolean) = CallAttributesCompat(
        displayName,
        Uri.parse("aspen://call/${state.channel}"),
        CallAttributesCompat.DIRECTION_OUTGOING,
        if (video) CallAttributesCompat.CALL_TYPE_VIDEO_CALL else CallAttributesCompat.CALL_TYPE_AUDIO_CALL,
        CallAttributesCompat.SUPPORTS_SET_INACTIVE,
    )

    /** Telecom has added the call: what was decided meanwhile is carried out on it. */
    private fun controlGiven(call: SystemCall, control: CallControlScope) {
        if (call.finished) {
            return
        }
        call.control = control
        call.following = scope.launch { followMute(control) }
        routes.attach(control)
        val endWanted = call.endWanted
        if (endWanted != null) {
            disconnect(call, endWanted)
        } else if (call.activeAsked) {
            setActive(call)
        }
    }

    /**
     * Telecom's own mute (a car's or a watch's button) is the engine's. The first value is the
     * state the call started in, not a change, and the engine's own mute is left as it is.
     */
    private suspend fun followMute(control: CallControlScope) {
        var last: Boolean? = null
        control.isMuted.collect { muted ->
            if (last != null && last != muted) {
                engine.setMuted(muted)
            }
            last = muted
        }
    }

    private fun connected(call: SystemCall, state: CallState) {
        val video = state.grants.camera || state.grants.shareScreen
        if (video && !call.video) {
            call.video = true
            call.control?.let { control ->
                scope.launch {
                    val result = control.requestCallType(CallAttributesCompat.CALL_TYPE_VIDEO_CALL)
                    if (result is CallControlResult.Error) {
                        Log.w(TAG, "Telecom kept the call to audio: error ${result.errorCode}")
                    }
                }
            }
        }
        if (!call.activeAsked) {
            call.activeAsked = true
            setActive(call)
        }
    }

    private fun setActive(call: SystemCall) {
        val control = call.control ?: return
        scope.launch {
            when (val result = control.setActive()) {
                is CallControlResult.Success -> if (call.phase == Phase.DIALING) call.phase = Phase.ACTIVE
                is CallControlResult.Error -> {
                    // Another call is active and cannot be held: ours waits, held, for Telecom
                    // to set it active when that one ends.
                    Log.w(TAG, "Telecom would not make the call active (error ${result.errorCode}); holding it")
                    held(call, held = true)
                }
            }
        }
    }

    /** Telecom ended the call (the notification's Hang up, a watch, a car): the engine's call ends with it. */
    private fun disconnectedByTelecom(call: SystemCall) {
        call.disconnectedByTelecom = true
        call.endedAtRevision = (snapshots.latest ?: engine.state()).revision
        engine.leave()
        finished(call)
    }

    /**
     * Mutes and deafens the user while Telecom holds the call (`onSetInactive`), and puts their own
     * mute and deafen back when it sets the call active again. The engine has no held state of its
     * own: others see the user muted and deafened, and a camera or screen the user shares goes on.
     */
    private fun held(call: SystemCall, held: Boolean) {
        val before = call.beforeHold
        if (held && before == null) {
            val state = snapshots.latest ?: engine.state()
            call.beforeHold = state.muted to state.deafened
            engine.setMuted(true)
            engine.setDeafened(true)
            call.phase = Phase.HELD
        } else if (!held && before != null) {
            call.beforeHold = null
            engine.setMuted(before.first)
            engine.setDeafened(before.second)
            call.phase = if (call.activeAsked) Phase.ACTIVE else Phase.DIALING
        }
    }

    private fun end(call: SystemCall, state: CallState, cause: DisconnectCause) {
        if (call.endWanted != null) {
            return
        }
        call.endWanted = cause
        call.endedAtRevision = state.revision
        if (call.control != null) {
            disconnect(call, cause)
        }
    }

    private fun disconnect(call: SystemCall, cause: DisconnectCause) {
        val control = call.control ?: return
        if (call.disconnectedByTelecom) {
            return
        }
        scope.launch {
            when (val result = control.disconnect(cause)) {
                is CallControlResult.Success -> finished(call)
                is CallControlResult.Error -> Log.w(TAG, "Telecom would not disconnect the call: error ${result.errorCode}")
            }
        }
    }

    /**
     * Telecom has let the call go (its disconnect done, or `addCall` over): everything held for it
     * is given back, once.
     */
    private fun finished(call: SystemCall) {
        if (call.finished) {
            return
        }
        call.finished = true
        call.following?.cancel()
        routes.detach()
        CallService.stop(context)
        audio.release()
        if (this.call === call) {
            this.call = null
        }
        // A call the engine joined after this one's end was asked, while Telecom was still letting
        // it go, is the system's call now.
        val latest = snapshots.latest
        val endedAt = call.endedAtRevision
        if (!closed && !call.refused && latest != null && latest.status.inCall && (endedAt == null || latest.revision > endedAt)) {
            begin(latest)
        }
    }

    /** Stops following the engine; a call under way is disconnected as the user's own leaving. */
    override fun close() {
        closed = true
        subscription.cancel()
        call?.let { end(it, snapshots.latest ?: engine.state(), DisconnectCause(DisconnectCause.LOCAL)) }
    }

    private suspend fun onMain(block: () -> Unit) = withContext(Dispatchers.Main.immediate) { block() }

    private val CallStatus.inCall: Boolean
        get() = this == CallStatus.JOINING || this == CallStatus.CONNECTED || this == CallStatus.REJOINING

    /**
     * The user's leaving is `LOCAL`; a call ended from elsewhere (removed by a moderator, access
     * lost, the server's idle end) is `REMOTE`; a failure is `ERROR`, which Telecom before SDK 38
     * reports as `LOCAL`.
     */
    private fun causeOf(state: CallState) = DisconnectCause(
        when {
            state.status == CallStatus.FAILED -> DisconnectCause.ERROR
            state.endedReason != null -> DisconnectCause.REMOTE
            else -> DisconnectCause.LOCAL
        },
    )

    companion object {
        private const val TAG = "AspenCall"
        private var manager: CallsManager? = null

        /** The process's one `CallsManager`, registered with Telecom as an app that can make video calls. */
        @Synchronized
        fun callsManager(context: Context): CallsManager = manager ?: CallsManager(context.applicationContext).also {
            it.registerAppWithTelecom(CallsManager.CAPABILITY_SUPPORTS_VIDEO_CALLING)
            manager = it
        }
    }
}
