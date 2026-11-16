package com.aspen_chat.client.call

import android.app.KeyguardManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.view.View
import android.view.WindowManager
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.content.ContextCompat
import com.aspen_chat.client.R
import com.aspen_chat.client.voice.CallState
import com.aspen_chat.client.voice.CallStatus

/**
 * The screen a DM's call ringing the phone shows over the lock screen: the incoming call
 * notification's full-screen intent opens it (`CallService.showIncoming`), and it turns the screen
 * on and shows over the keyguard (`turnScreenOn` and `showWhenLocked` in the manifest), as a phone
 * call's screen does, which the app's own activity must never do. It shows the caller's name and
 * the deployment, and Answer and Decline, which run through `CallService` as the notification's
 * actions do, so a ring is answered and declined on one path however it is answered.
 *
 * It follows the ring through `CallTelecom`'s phase. While it is `RINGING` the screen shows the
 * ring and keeps the display on. When the phase leaves it for `IDLE`, the ring ended, here or
 * elsewhere, and the screen closes. When it leaves it for a call (answered here, from the
 * notification, or by a headset through Telecom), the call needs the app, whose web view shows it,
 * and that cannot show over the lock screen: on a phone that is unlocked the app is opened at once;
 * on a locked one this screen stays as the call's, with the call's state and Hang up, until the
 * phone is unlocked, by the user on their own (`ACTION_USER_PRESENT`) or through Open Aspen, which
 * asks the keyguard to be dismissed (`KeyguardManager.requestDismissKeyguard`, the system's own
 * unlock), and then opens the app. The keyguard is never asked to go for the ring itself.
 *
 * Its task is its own (`singleInstance`, out of the recents), so it never sits in the app's task
 * and finishing it leaves whatever was on the screen before. A ring for another deployment while it
 * shows reaches `onNewIntent` and rebinds it.
 */
class IncomingCallActivity : AppCompatActivity() {
    private lateinit var caller: TextView
    private lateinit var status: TextView
    private lateinit var ringingActions: View
    private lateinit var callActions: View
    private var engine: CallEngine? = null
    private var phases: CallTelecom.Subscription? = null
    private var snapshots: CallSnapshots.Subscription? = null
    /** Whether the ring this screen showed became a call. */
    private var answered = false
    private var unlockedRegistered = false

    private val unlocked = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (answered) {
                handOver()
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
            setShowWhenLocked(true)
            setTurnScreenOn(true)
        } else {
            @Suppress("DEPRECATION")
            window.addFlags(WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED or WindowManager.LayoutParams.FLAG_TURN_SCREEN_ON)
        }
        setContentView(R.layout.activity_incoming_call)
        caller = findViewById(R.id.incoming_call_caller)
        status = findViewById(R.id.incoming_call_status)
        ringingActions = findViewById(R.id.incoming_call_ringing_actions)
        callActions = findViewById(R.id.incoming_call_call_actions)
        findViewById<View>(R.id.incoming_call_answer).setOnClickListener { CallService.answer(this) }
        findViewById<View>(R.id.incoming_call_decline).setOnClickListener { CallService.decline(this) }
        findViewById<View>(R.id.incoming_call_hang_up).setOnClickListener { CallService.hangUp(this) }
        findViewById<View>(R.id.incoming_call_open_app).setOnClickListener { askToUnlock() }
        ContextCompat.registerReceiver(this, unlocked, IntentFilter(Intent.ACTION_USER_PRESENT), ContextCompat.RECEIVER_NOT_EXPORTED)
        unlockedRegistered = true
        bind(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        bind(intent)
    }

    /** Shows the call of the deployment the intent names; a stale intent, with no call to show, closes the screen. */
    private fun bind(intent: Intent) {
        unbind()
        val origin = intent.getStringExtra(EXTRA_ORIGIN)
        val engine = origin?.let { CallEngines.existing(it) }
        if (engine == null || engine.telecom.phase == CallTelecom.Phase.IDLE) {
            finish()
            return
        }
        this.engine = engine
        answered = false
        caller.text = engine.telecom.displayName
        status.text = getString(R.string.aspen_incoming_call_at, Uri.parse(origin).authority ?: origin)
        phases = engine.telecom.listen { phase -> phaseChanged(phase) }
        snapshots = engine.snapshots.listen { state -> stateChanged(state) }
    }

    private fun unbind() {
        phases?.cancel()
        phases = null
        snapshots?.cancel()
        snapshots = null
        engine = null
    }

    private fun phaseChanged(phase: CallTelecom.Phase) {
        when (phase) {
            CallTelecom.Phase.RINGING -> {
                window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                ringingActions.visibility = View.VISIBLE
                callActions.visibility = View.GONE
            }
            CallTelecom.Phase.IDLE -> finish()
            CallTelecom.Phase.DIALING, CallTelecom.Phase.ACTIVE, CallTelecom.Phase.HELD -> {
                answered = true
                if (!keyguard().isKeyguardLocked) {
                    handOver()
                    return
                }
                // The display may go dark on its own again now, as in any call.
                window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
                ringingActions.visibility = View.GONE
                callActions.visibility = View.VISIBLE
                engine?.snapshots?.latest?.let { stateChanged(it) }
            }
        }
    }

    /** The call's state under the caller's name, once the ring is a call. */
    private fun stateChanged(state: CallState) {
        if (!answered) {
            return
        }
        status.text = getString(
            when {
                state.held -> R.string.aspen_call_on_hold
                state.status == CallStatus.CONNECTED -> R.string.aspen_call_in_progress
                else -> R.string.aspen_call_connecting
            },
        )
    }

    private fun keyguard(): KeyguardManager = getSystemService(KeyguardManager::class.java)

    /** Asks the system to unlock the phone, as the user would; the app opens once it has. */
    private fun askToUnlock() {
        keyguard().requestDismissKeyguard(
            this,
            object : KeyguardManager.KeyguardDismissCallback() {
                override fun onDismissSucceeded() {
                    handOver()
                }
            },
        )
    }

    /** Opens the app, which shows the call from here on, and closes this screen. */
    private fun handOver() {
        val open = packageManager.getLaunchIntentForPackage(packageName)
        if (open != null) {
            open.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
            startActivity(open)
        }
        finish()
    }

    override fun onDestroy() {
        unbind()
        if (unlockedRegistered) {
            unregisterReceiver(unlocked)
            unlockedRegistered = false
        }
        super.onDestroy()
    }

    companion object {
        private const val EXTRA_ORIGIN = "origin"

        /** The intent that shows the ring of the deployment at `origin` (its `public_url`). */
        fun intent(context: Context, origin: String): Intent =
            Intent(context, IncomingCallActivity::class.java)
                .putExtra(EXTRA_ORIGIN, origin)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }
}
