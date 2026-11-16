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
 * once the app has left the screen: a foreground service of the two kinds a call is (a phone
 * call, and the microphone), posting the ongoing call's notification (`CallStyle`, with the
 * system's own Hang up) the moment it starts. `CallTelecom` starts it as it adds the call to
 * Telecom and stops it when the call ends, so the notification is up within the five seconds
 * Telecom allows, after which (from Android 14) Telecom itself keeps the process alive for a
 * call it holds, and this service's type says what the process is doing.
 *
 * The microphone kind may be refused where the app is not in front when the service starts
 * (Android 14 lets a service record only from the foreground); the service then runs as a phone
 * call alone, and the call's microphone is Telecom's to grant.
 */
class CallService : Service() {
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_HANG_UP) {
            // The notification's Hang up, which leaves the call through the engine; the call's
            // end then stops the service. A process started again for a stale notification has
            // no call to hang up, and stops.
            val hangUp = synchronized(CallService) { Companion.hangUp }
            if (hangUp == null) finish() else hangUp()
            return START_NOT_STICKY
        }
        val name = intent?.getCharSequenceExtra(EXTRA_NAME) ?: getString(R.string.app_name)
        val video = intent?.getBooleanExtra(EXTRA_VIDEO, false) ?: false
        enterForeground(notification(this, name, video))
        return START_NOT_STICKY
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun enterForeground(notification: Notification) {
        val call = ServiceInfo.FOREGROUND_SERVICE_TYPE_PHONE_CALL
        val microphone = ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
        try {
            try {
                ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, call or microphone)
            } catch (refused: SecurityException) {
                Log.w(TAG, "the call's service may not record; it runs as a phone call alone", refused)
                ServiceCompat.startForeground(this, NOTIFICATION_ID, notification, call)
            }
        } catch (e: RuntimeException) {
            // From Android 12 a service started in the background may not enter the foreground
            // (`ForegroundServiceStartNotAllowedException`); the call goes on under Telecom.
            Log.w(TAG, "the call goes on without a foreground service", e)
            finish()
        }
    }

    private fun finish() {
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    companion object {
        private const val TAG = "AspenCall"
        const val CHANNEL = "aspen_calls"
        const val NOTIFICATION_ID = 0x43414c4c
        private const val ACTION_HANG_UP = "com.aspen_chat.client.call.HANG_UP"
        private const val EXTRA_NAME = "name"
        private const val EXTRA_VIDEO = "video"
        private var hangUp: (() -> Unit)? = null

        /**
         * Starts the service for a call shown as `name`, whose notification's Hang up runs
         * `hangUp`. Where the system refuses to start it (the app in the background on Android 12
         * and later, with no call yet in Telecom to allow it), the call goes on without it.
         */
        fun start(context: Context, name: CharSequence, video: Boolean, hangUp: () -> Unit) {
            synchronized(this) { this.hangUp = hangUp }
            val intent = Intent(context, CallService::class.java)
                .putExtra(EXTRA_NAME, name)
                .putExtra(EXTRA_VIDEO, video)
            try {
                ContextCompat.startForegroundService(context, intent)
            } catch (e: IllegalStateException) {
                Log.w(TAG, "the call's service could not be started", e)
            }
        }

        fun stop(context: Context) {
            synchronized(this) { hangUp = null }
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

            val hangUp = PendingIntent.getService(
                context,
                0,
                Intent(context, CallService::class.java).setAction(ACTION_HANG_UP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
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
