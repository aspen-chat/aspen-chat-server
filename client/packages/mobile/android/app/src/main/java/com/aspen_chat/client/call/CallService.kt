package com.aspen_chat.client.call

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import com.aspen_chat.client.R

/**
 * Keeps the process alive for the length of a call the call engine holds, so the call goes on
 * once the app has left the screen: a foreground service of the kinds a call is (a phone call,
 * the microphone, and the camera while the call's camera is on), posting the ongoing call's
 * notification (`CallStyle`, with the system's own Hang up) the moment it starts. `CallTelecom`
 * starts it as it adds the call to Telecom and stops it when the call ends, so the notification
 * is up within the five seconds Telecom allows, after which (from Android 14) Telecom itself keeps
 * the process alive for a call it holds, and this service's type says what the process is doing;
 * `setCamera` adds the camera's kind as the camera turns on and takes it off as it turns off, so
 * the camera may go on while the app is not in front, and `setProjection` the media projection's
 * kind the same way for a share of the screen, which the system refuses to start without it
 * (from Android 14) and which, under it, goes on while the app is not in front.
 *
 * The microphone and camera kinds may be refused where the app is not in front when the service
 * takes them (Android 14 lets a service start recording only from the foreground); the service
 * then runs without them, and the call's microphone is Telecom's to grant. `setProjection` is
 * called from the consent's answer, with the app in front, and its listener hears once the
 * service runs with the kind, which is when a projection may be made.
 *
 * A DM's call ringing the phone is shown by `showIncoming`: a `CallStyle` incoming call
 * notification with the system's Answer and Decline, posted directly rather than by a service
 * (its full-screen intent is what Android 12 asks of a call notification that is no foreground
 * service's), whose full-screen intent opens `IncomingCallActivity` over the lock screen. Its
 * channel makes no sound of its own: the ring is heard through `CallRinger`, which `CallTelecom`
 * plays for the ring's length, since a notification's sound plays once. Its actions come back here
 * and reach `CallTelecom`, as `IncomingCallActivity`'s do through `answer` and `decline`, so a ring
 * is answered and declined on one path however it is. Answering starts the service as for any
 * call, whose ongoing notification replaces the incoming one.
 */
class CallService : Service() {
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_HANG_UP -> {
                // The notification's Hang up, which leaves the call through the engine; the
                // call's end then stops the service. A process started again for a stale
                // notification has no call to hang up, and stops.
                val hangUp = synchronized(CallService) { Companion.hangUp }
                if (hangUp == null) finish() else hangUp()
                return START_NOT_STICKY
            }
            ACTION_ANSWER, ACTION_DECLINE -> {
                // The incoming call's Answer or Decline; a stale notification's finds no ring.
                val ring = synchronized(CallService) { Companion.ring }
                if (ring == null) {
                    getSystemService(NotificationManager::class.java).cancel(NOTIFICATION_ID)
                    finish()
                } else if (intent.action == ACTION_ANSWER) {
                    ring.answer()
                } else {
                    ring.decline()
                }
                return START_NOT_STICKY
            }
            else -> {}
        }
        val name = intent?.getCharSequenceExtra(EXTRA_NAME) ?: getString(R.string.app_name)
        val video = intent?.getBooleanExtra(EXTRA_VIDEO, false) ?: false
        val camera = intent?.getBooleanExtra(EXTRA_CAMERA, false) ?: false
        val projection = intent?.getBooleanExtra(EXTRA_PROJECTION, false) ?: false
        enterForeground(notification(this, name, video), camera, projection)
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun enterForeground(notification: Notification, camera: Boolean, projection: Boolean) {
        val call = ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL
        val microphone = ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        val cameraType = if (camera) ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA else 0
        val projectionType = if (projection) ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION else 0
        var projecting = false
        try {
            try {
                ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, call or microphone or cameraType or projectionType)
                synchronized(CallService) { types = call or microphone or cameraType or projectionType }
                projecting = projection
            } catch (refused: SecurityException) {
                Log.w(TAG, "the call's service may not record or capture; it runs as a phone call alone", refused)
                ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, call)
                synchronized(CallService) { types = call }
            }
        } catch (e: RuntimeException) {
            // From Android 12 a service started in the background may not enter the foreground
            // (`ForegroundServiceStartNotAllowedException`); the call goes on under Telecom.
            Log.w(TAG, "the call goes on without a foreground service", e)
            finish()
        }
        if (projection) {
            val listener = synchronized(CallService) { projectionListener.also { projectionListener = null } }
            listener?.invoke(projecting)
        }
    }

    private fun finish() {
        synchronized(CallService) { types = 0 }
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        synchronized(CallService) { types = 0 }
        super.onDestroy()
    }

    /** What the incoming call's notification does; `answer` and `decline` run on the caller's thread. */
    private class Ringing(val answer: () -> Unit, val decline: () -> Unit)

    companion object {
        private const val TAG = "AspenCall"
        const val CHANNEL = "aspen_calls"
        /** The incoming calls' channel; neither it nor `CHANNEL` makes a sound (see `CallRinger`). */
        const val RING_CHANNEL = "aspen_incoming_calls"
        const val NOTIFICATION_ID = 0x43414c4c
        private const val ACTION_HANG_UP = "com.aspen_chat.client.call.HANG_UP"
        private const val ACTION_ANSWER = "com.aspen_chat.client.call.ANSWER"
        private const val ACTION_DECLINE = "com.aspen_chat.client.call.DECLINE"
        private const val EXTRA_NAME = "name"
        private const val EXTRA_VIDEO = "video"
        private const val EXTRA_CAMERA = "camera"
        private const val EXTRA_PROJECTION = "projection"
        private var hangUp: (() -> Unit)? = null
        /** Told whether the service runs with the media projection's kind, once it has been asked for. */
        private var projectionListener: ((Boolean) -> Unit)? = null
        private var camera = false
        private var projection = false

        /**
         * The foreground service types the service runs with now (`ServiceInfo
         * .FOREGROUND_SERVICE_TYPE_*`), 0 while it is not in the foreground, for the proof.
         */
        var types = 0
            private set
        /** What the service was last started with, so the camera's kind can be added to the same call. */
        private var shown: Pair<CharSequence, Boolean>? = null
        private var ring: Ringing? = null

        /**
         * Shows a DM's call from `name` ringing the phone: the incoming call's notification, with
         * the system's Answer and Decline, a full-screen intent that opens the incoming call screen
         * for the ring's deployment over the lock screen, and a tap that opens the app. The same
         * notification id as the ongoing call's, which replaces it on answering.
         */
        fun showIncoming(context: Context, origin: String, name: CharSequence, answer: () -> Unit, decline: () -> Unit) {
            synchronized(this) { ring = Ringing(answer, decline) }
            val manager = context.getSystemService(NotificationManager::class.java)
            val channel = NotificationChannel(RING_CHANNEL, context.getString(R.string.aspen_rings_channel_name), NotificationManager.IMPORTANCE_HIGH)
            // The ring is `CallRinger`'s, for as long as the call rings; a sound here would play
            // once more beside it.
            channel.setSound(null, null)
            channel.enableVibration(false)
            manager.createNotificationChannel(channel)
            val builder = NotificationCompat.Builder(context, RING_CHANNEL)
                .setSmallIcon(R.drawable.ic_stat_aspen)
                .setColor(0xFF047857.toInt())
                .setContentTitle(name)
                .setContentText(context.getString(R.string.aspen_incoming_call))
                .setStyle(
                    NotificationCompat.CallStyle.forIncomingCall(
                        Person.Builder().setName(name).setImportant(true).build(),
                        declineIntent(context),
                        answerIntent(context),
                    ),
                )
                .setCategory(NotificationCompat.CATEGORY_CALL)
                .setPriority(NotificationCompat.PRIORITY_MAX)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
            val open = context.packageManager.getLaunchIntentForPackage(context.packageName)
            if (open != null) {
                open.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                builder.setContentIntent(PendingIntent.getActivity(context, 0, open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
            }
            builder.setFullScreenIntent(
                PendingIntent.getActivity(context, 3, IncomingCallActivity.intent(context, origin), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE),
                true,
            )
            manager.notify(NOTIFICATION_ID, builder.build())
        }

        /**
         * The incoming call's Answer and Decline, as the notification carries them. The system
         * gives one `PendingIntent` for each request code and intent, so a test that builds
         * either again finds the notification's action by equality, never by its wording.
         */
        internal fun answerIntent(context: Context): PendingIntent = ringAction(context, ACTION_ANSWER, 2)

        internal fun declineIntent(context: Context): PendingIntent = ringAction(context, ACTION_DECLINE, 1)

        private fun ringAction(context: Context, name: String, request: Int): PendingIntent = PendingIntent.getService(
            context,
            request,
            action(context, name),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )

        private fun action(context: Context, name: String): Intent = Intent(context, CallService::class.java).setAction(name)

        /**
         * Answers the ring, declines it, or hangs the call up, as the notifications' actions do:
         * the same intents, started by whoever is in the foreground (the incoming call screen), so
         * each runs on the one path `onStartCommand` holds.
         */
        fun answer(context: Context) {
            context.startService(action(context, ACTION_ANSWER))
        }

        fun decline(context: Context) {
            context.startService(action(context, ACTION_DECLINE))
        }

        fun hangUp(context: Context) {
            context.startService(action(context, ACTION_HANG_UP))
        }

        /** Takes the incoming call's notification down, whatever ended the ring. */
        fun cancelIncoming(context: Context) {
            synchronized(this) { ring = null }
            context.getSystemService(NotificationManager::class.java).cancel(NOTIFICATION_ID)
        }

        /**
         * Starts the service for a call shown as `name`, whose notification's Hang up runs
         * `hangUp`. Where the system refuses to start it (the app in the background on Android 12
         * and later, with no call yet in Telecom to allow it), the call goes on without it.
         */
        fun start(context: Context, name: CharSequence, video: Boolean, hangUp: () -> Unit) {
            synchronized(this) {
                this.hangUp = hangUp
                shown = name to video
                camera = false
                projection = false
            }
            send(context, name, video, camera = false, projection = false)
        }

        /**
         * Adds the camera's kind to the running service as the call's camera turns on, and takes
         * it off as it turns off; nothing where no call's service runs.
         */
        fun setCamera(context: Context, on: Boolean) {
            val (name, video, projecting) = synchronized(this) {
                camera = on
                val (name, video) = shown ?: return
                Triple(name, video, projection)
            }
            send(context, name, video, on, projecting)
        }

        /**
         * Adds the media projection's kind as a share of the screen is consented to, telling
         * `listener` whether the service then runs with it (so a projection may be made), and
         * takes it off as the share ends; `listener` hears `false` where no call's service runs.
         */
        fun setProjection(context: Context, on: Boolean, listener: ((Boolean) -> Unit)? = null) {
            val (name, video, cameraOn) = synchronized(this) {
                projection = on
                val (name, video) = shown ?: run {
                    projectionListener = null
                    return@synchronized null
                }
                projectionListener = if (on) listener else null
                Triple(name, video, camera)
            } ?: run {
                listener?.invoke(false)
                return
            }
            send(context, name, video, cameraOn, on)
        }

        private fun send(context: Context, name: CharSequence, video: Boolean, camera: Boolean, projection: Boolean) {
            val intent = Intent(context, CallService::class.java)
                .putExtra(EXTRA_NAME, name)
                .putExtra(EXTRA_VIDEO, video)
                .putExtra(EXTRA_CAMERA, camera)
                .putExtra(EXTRA_PROJECTION, projection)
            try {
                ContextCompat.startForegroundService(context, intent)
            } catch (e: IllegalStateException) {
                Log.w(TAG, "the call's service could not be started", e)
            }
        }

        fun stop(context: Context) {
            synchronized(this) {
                hangUp = null
                shown = null
                camera = false
                projection = false
                projectionListener = null
            }
            context.stopService(Intent(context, CallService::class.java))
        }

        /** The ongoing call's notification, a `CallStyle` whose Hang up comes back to the service. */
        fun notification(context: Context, name: CharSequence, video: Boolean): Notification {
            val manager = context.getSystemService(NotificationManager::class.java)
            val channel = NotificationChannel(CHANNEL, context.getString(R.string.aspen_calls_channel_name), NotificationManager.IMPORTANCE_DEFAULT)
            // A call under way makes no sound of its own; its channel is what lets the system
            // show it prominently.
            channel.setSound(null, null)
            channel.enableVibration(false)
            manager.createNotificationChannel(channel)

            val hangUp = PendingIntent.getService(context, 0, action(context, ACTION_HANG_UP), PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            val builder = NotificationCompat.Builder(context, CHANNEL)
                // The status bar draws a small icon by its alpha alone; the colour tints it where shown.
                .setSmallIcon(R.drawable.ic_stat_aspen)
                .setColor(0xFF047857.toInt())
                // The title before Android 12, where there is no call style; the style shows the
                // person's name over it from then on.
                .setContentTitle(name)
                .setContentText(context.getString(R.string.aspen_call_in_progress))
                .setStyle(NotificationCompat.CallStyle.forOngoingCall(Person.Builder().setName(name).setImportant(true).build(), hangUp).setIsVideo(video))
                .setCategory(NotificationCompat.CATEGORY_CALL)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            val open = context.packageManager.getLaunchIntentForPackage(context.packageName)
            if (open != null) {
                open.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP)
                builder.setContentIntent(PendingIntent.getActivity(context, 0, open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE))
            }
            return builder.build()
        }
    }
}
