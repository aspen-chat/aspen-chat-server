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
import com.aspen_chat.client.voice.Ring
import com.aspen_chat.client.voice.RingEnd
import com.aspen_chat.client.voice.RingObserver
import com.aspen_chat.client.voice.RingOptions
import com.aspen_chat.client.voice.RingPlatform
import com.aspen_chat.client.voice.RingWatch
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
 * tells it of) is the engine's hold until it sets the call active again (`onSetActive`), and a
 * mute from the system is the engine's mute.
 *
 * A DM's call ringing this phone (`ring`, from a `call` push through `CallRings`) is an incoming
 * call in Telecom before the engine joins, with the incoming call's notification and its
 * full-screen intent (`CallService.showIncoming`, opening `IncomingCallActivity` over the lock
 * screen), the ringtone and vibration (`CallRinger`, since Telecom rings for no call an app manages
 * itself), and the engine's ring watch on it. Answering (Telecom's `onAnswer`, or `CallService`'s
 * Answer, from the notification or the screen) joins the ring's channel through the engine, which
 * ends the ring at the deployment, and the call goes on as any other; declining (Telecom's
 * disconnect, or `CallService`'s Decline) ends the ring at the deployment (`CallRings.decline`);
 * and the ring ending anywhere else (`ringEnded`) disconnects the call with the cause its end
 * gives. Whoever shows the call follows `phase` through `listen`.
 */
class CallTelecom(
    context: Context,
    private val engine: Engine,
    private val snapshots: CallSnapshots,
    private val audio: CallAudio,
    private val routes: CallRoutes,
) : AutoCloseable {
    /** Where the system's call stands; `IDLE` while the engine is in none and nothing rings. */
    enum class Phase { IDLE, RINGING, DIALING, ACTIVE, HELD }

    private val context = context.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val ringer = CallRinger(context)
    private var call: SystemCall? = null
    private var closed = false
    private val subscription = snapshots.listen(::stateChanged)
    private val listeners = ArrayList<PhaseListener>()
    /** The phase the listeners were last told, so a change that lands on the same phase tells them nothing. */
    private var announced = Phase.IDLE

    fun interface PhaseListener {
        fun changed(phase: Phase)
    }

    /** Stops the listener it was given for. */
    inner class Subscription internal constructor(private val listener: PhaseListener) {
        fun cancel() {
            listeners.remove(listener)
        }
    }

    /**
     * What Telecom and the notification show the call as: the channel's or the other person's
     * name, which the interface that commands the engine sets before joining. The engine's
     * snapshots name the channel by id alone, so until it is set the call is shown as the app.
     */
    var displayName: CharSequence = context.getString(R.string.app_name)

    val phase: Phase
        get() = call?.phase ?: Phase.IDLE

    /**
     * Calls `listener` on the main thread with the phase as it is and with every change of it,
     * until the subscription is cancelled. Call on the main thread.
     */
    fun listen(listener: PhaseListener): Subscription {
        listeners.add(listener)
        val now = phase
        scope.launch { if (listeners.contains(listener)) listener.changed(now) }
        return Subscription(listener)
    }

    /** Tells the listeners of the phase when it has changed since they were last told. */
    private fun announce() {
        val now = phase
        if (now == announced) {
            return
        }
        announced = now
        for (listener in listeners.toList()) {
            listener.changed(now)
        }
    }

    /** Telecom's id for the call, once it has added it; what `dumpsys telecom` lists it by. */
    val callId: ParcelUuid?
        get() = call?.control?.getCallId()

    /**
     * One call in Telecom, from `addCall` until Telecom has let it go. Read and written on the
     * main thread.
     */
    private inner class SystemCall(var video: Boolean, val incoming: CallRings.IncomingRing? = null) {
        var phase = if (incoming == null) Phase.DIALING else Phase.RINGING
            set(value) {
                field = value
                if (call === this) announce()
            }
        var control: CallControlScope? = null
        /** The engine's watch on the ring, while the call is incoming and unanswered. */
        var watch: RingWatch? = null
        /** Whether the incoming call was answered here, so the engine's join is this call's. */
        var answered = false
        /** Whether answering opened the microphone, which the call's end then closes. */
        var openedMicrophone = false
        /**
         * Whether the engine has been in this call: from the start for a call made from the
         * engine's own snapshot, and for an answered ring from its first snapshot in the call.
         * Until then an `idle` snapshot is not the call's end but the engine's state as it was
         * (the microphone given at answering publishes one before the join).
         */
        var joined = incoming == null
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
    }

    private fun stateChanged(state: CallState) {
        val call = call
        when {
            closed -> {}
            call == null && state.status.inCall -> begin(state)
            call == null -> {}
            // A ring stands apart from the engine's state until it is answered here.
            call.phase == Phase.RINGING -> {}
            state.status.inCall -> {
                call.joined = true
                if (state.status == CallStatus.CONNECTED) {
                    connected(call, state)
                }
            }
            // `IDLE` or `FAILED`: the call's end once the engine has been in it.
            call.joined -> end(call, state, causeOf(state))
        }
    }

    /**
     * A DM's call rings the phone: added to Telecom as an incoming call, shown with the incoming
     * call's notification, and watched through the engine until it is answered, declined, or ends
     * elsewhere. One call at a time: a ring while the engine is in a call, or another rings, is
     * declined nowhere and shown nowhere here (the person's other devices show it).
     */
    fun ring(ring: CallRings.IncomingRing) {
        if (closed || call != null) {
            Log.i(TAG, "a call rang while another call is held; not shown")
            return
        }
        val call = SystemCall(video = false, incoming = ring)
        this.call = call
        displayName = ring.callerName.ifEmpty { context.getString(R.string.app_name) }
        CallService.showIncoming(
            context,
            ring.account,
            displayName,
            answer = { onMainLater { answer(call, byTelecom = false) } },
            decline = { onMainLater { declined(call, byTelecom = false) } },
        )
        ringer.start()
        announce()
        call.watch = RingWatch.start(
            RingPlatform(CallHttp(), CallSockets()),
            RingOptions(ring.account.origin, CallCredentials(context, ring.account), CallEngines.loopbackPlainText(context)),
            Ring(ring.channel, ring.session, ring.until),
            object : RingObserver {
                override fun ended(end: RingEnd) {
                    onMainLater { ringEnded(call, end) }
                }
            },
        )
        val attributes = CallAttributesCompat(
            displayName,
            Uri.parse("aspen://call/${ring.channel}"),
            CallAttributesCompat.DIRECTION_INCOMING,
            CallAttributesCompat.CALL_TYPE_AUDIO_CALL,
            CallAttributesCompat.SUPPORTS_SET_INACTIVE,
        )
        scope.launch {
            try {
                callsManager(context).addCall(
                    attributes,
                    onAnswer = { onMain { answer(call, byTelecom = true) } },
                    onDisconnect = { onMain { declined(call, byTelecom = true) } },
                    onSetActive = { onMain { held(call, held = false) } },
                    onSetInactive = { onMain { held(call, held = true) } },
                ) {
                    val control = this
                    scope.launch { controlGiven(call, control) }
                }
            } catch (refused: CallException) {
                Log.w(TAG, "Telecom refused the incoming call (error ${refused.code})", refused)
                call.refused = true
                stopRinging(call)
            } finally {
                finished(call)
            }
        }
    }

    /**
     * The ring is answered here, by the notification or by Telecom itself (a headset's button,
     * which has made the call active already): the engine joins the DM's call, which ends the
     * ring at the deployment, and the call goes on as an active call of Telecom's, with the
     * ongoing call's notification in place of the incoming one. No interface commands the engine
     * at a ring answered from the lock screen, so the microphone is opened here: the engine
     * refuses to join a call that lets the user speak without one.
     */
    private fun answer(call: SystemCall, byTelecom: Boolean) {
        val ring = call.incoming ?: return
        if (call.phase != Phase.RINGING || call.finished) {
            return
        }
        stopRinging(call)
        call.answered = true
        call.phase = Phase.DIALING
        // Answering is what makes the call active; the engine's connection need not ask again.
        call.activeAsked = true
        audio.configureForCall()
        if (engine.state().microphone == null) {
            engine.setMicrophone(audio.openMicrophone())
            call.openedMicrophone = true
        }
        CallService.start(context, displayName, call.video) { engine.leave() }
        if (byTelecom) {
            call.phase = Phase.ACTIVE
        } else {
            call.control?.let { control -> scope.launch { answerOn(call, control) } }
        }
        engine.join(ring.channel)
    }

    /** Answers the call in Telecom, which makes it active there; the phase follows once it has. */
    private suspend fun answerOn(call: SystemCall, control: CallControlScope) {
        when (val result = control.answer(CallAttributesCompat.CALL_TYPE_AUDIO_CALL)) {
            is CallControlResult.Success -> if (call.phase == Phase.DIALING) call.phase = Phase.ACTIVE
            is CallControlResult.Error -> Log.w(TAG, "Telecom would not answer the call: error ${result.errorCode}")
        }
    }

    /**
     * The ring is declined here, by the notification or by Telecom (the system's own UI, a
     * headset): the deployment is told, so the person's other devices stop too. Telecom's own
     * disconnect of an answered call is the user's leaving, as for any call.
     */
    private fun declined(call: SystemCall, byTelecom: Boolean) {
        val ring = call.incoming
        if (ring == null || call.phase != Phase.RINGING || call.finished) {
            if (byTelecom) {
                disconnectedByTelecom(call)
            }
            return
        }
        stopRinging(call)
        CallRings.decline(context, ring.account, ring.channel)
        call.endedAtRevision = (snapshots.latest ?: engine.state()).revision
        val cause = DisconnectCause(DisconnectCause.REJECTED)
        when {
            byTelecom -> {
                call.disconnectedByTelecom = true
                finished(call)
            }
            call.control != null -> disconnect(call, cause)
            else -> call.endWanted = cause
        }
    }

    /**
     * The ring ended anywhere else, as the engine's watch says: Telecom's call ends with the
     * cause, one of the four a call added through `CallsManager` may be disconnected with
     * (`LOCAL`, `REMOTE`, `MISSED`, `REJECTED`; `CallControl` refuses every other code): answered
     * on another device or ended by its caller, the call went on or ended beyond this phone
     * (`REMOTE`); declined elsewhere, it was rejected; run out, it was missed; and a ring whose
     * account has no session any more ends by this phone's own doing.
     */
    private fun ringEnded(call: SystemCall, end: RingEnd) {
        if (call.phase != Phase.RINGING || call.finished) {
            return
        }
        stopRinging(call)
        call.endedAtRevision = (snapshots.latest ?: engine.state()).revision
        val cause = when (end) {
            RingEnd.ANSWERED_ELSEWHERE, RingEnd.CALL_ENDED -> DisconnectCause.REMOTE
            RingEnd.DECLINED -> DisconnectCause.REJECTED
            RingEnd.TIMED_OUT -> DisconnectCause.MISSED
            RingEnd.SIGNED_OUT -> DisconnectCause.LOCAL
        }
        if (call.control != null) {
            disconnect(call, DisconnectCause(cause))
        } else {
            call.endWanted = DisconnectCause(cause)
        }
    }

    /** The ringtone, the incoming call's notification, and the ring watch are done with, whatever ends the ring. */
    private fun stopRinging(call: SystemCall) {
        ringer.stop()
        call.watch?.stop()
        call.watch = null
        CallService.cancelIncoming(context)
    }

    private fun begin(state: CallState) {
        val video = state.grants.camera || state.grants.shareScreen
        val call = SystemCall(video)
        this.call = call
        announce()
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
        } else if (call.answered) {
            scope.launch { answerOn(call, control) }
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
     * Telecom's hold (`onSetInactive`) is the engine's: the voice server pauses everything the
     * user sends, a camera or screen included, sends them nothing, and keeps their own mute and
     * deafen for when Telecom sets the call active again (`onSetActive`); the others see them on
     * hold rather than muted.
     */
    private fun held(call: SystemCall, held: Boolean) {
        if (held == (call.phase == Phase.HELD)) {
            return
        }
        engine.setHeld(held)
        call.phase = when {
            held -> Phase.HELD
            call.activeAsked -> Phase.ACTIVE
            else -> Phase.DIALING
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
        stopRinging(call)
        call.following?.cancel()
        routes.detach()
        CallService.stop(context)
        if (call.openedMicrophone) {
            engine.setMicrophone(null)
            audio.closeMicrophone()
        }
        audio.release()
        if (this.call === call) {
            this.call = null
            announce()
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

    /** Runs `block` on the main thread from any other: a notification's action, the watch's thread. */
    private fun onMainLater(block: () -> Unit) {
        scope.launch { block() }
    }

    private val CallStatus.inCall: Boolean
        get() = this == CallStatus.JOINING || this == CallStatus.CONNECTED || this == CallStatus.REJOINING

    /**
     * The user's leaving, and a failure on this side, are `LOCAL`; a call ended from elsewhere
     * (removed by a moderator, access lost, the server's idle end) is `REMOTE`. A call added
     * through `CallsManager` is disconnected with one of `LOCAL`, `REMOTE`, `MISSED`, and
     * `REJECTED` alone; `CallControl` refuses `ERROR` and every other code.
     */
    private fun causeOf(state: CallState) = DisconnectCause(
        if (state.endedReason != null && state.status != CallStatus.FAILED) DisconnectCause.REMOTE else DisconnectCause.LOCAL,
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
