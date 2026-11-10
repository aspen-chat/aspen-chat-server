package com.aspen_chat.client.uploads;

import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;
import android.util.Log;
import androidx.core.app.ServiceCompat;

/**
 * Keeps the process alive, and its processor awake, while uploads are under way, so that they
 * go on once the app has left the screen: a foreground service for as long as there is
 * something to send, with the notification the system requires of one. The uploads themselves
 * are the process's ({@link DeviceUploads}), not the service's, and go on without it where the
 * system will not let it start or stay in the foreground, for as long as the process lasts.
 *
 * It is sticky: the system starts it again after killing the process, and each start sends
 * whatever the records say was still being sent.
 */
public class UploadService extends Service implements Uploads.Listener {

    private static final String TAG = "AspenUploads";
    /**
     * The longest the processor is held awake for one start. The system allows a service that
     * syncs data six hours in a day, so nothing here works for longer.
     */
    private static final long AWAKE_MILLISECONDS = 6L * 60 * 60 * 1000;

    private final Handler main = new Handler(Looper.getMainLooper());
    private Uploads uploads;
    private PowerManager.WakeLock awake;

    /** Starts the service where the system allows; the uploads go on in the process regardless. */
    static void start(Context context) {
        Intent intent = new Intent(context, UploadService.class);
        try {
            try {
                // Not `startForegroundService`: it obliges the service to enter the
                // foreground, and crashes the app when the system then refuses that (a day's
                // allowance used up), while a service started plainly may enter it or not.
                context.startService(intent);
            } catch (IllegalStateException background) {
                // The app is not in front, where only a foreground service may be started;
                // and from Android 12 not even that.
                if (Build.VERSION.SDK_INT < Build.VERSION_CODES.O || Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    throw background;
                }
                context.startForegroundService(intent);
            }
        } catch (RuntimeException e) {
            Log.w(TAG, "uploads go on without their service", e);
        }
    }

    @Override
    public void onCreate() {
        super.onCreate();
        uploads = DeviceUploads.of(this);
        uploads.listen(this);
        awake = getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "aspen:uploads");
        awake.setReferenceCounted(false);
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        uploads.resume();
        // Entered even with nothing to send: a start that promised the foreground must keep
        // the promise before the service may stop.
        enterForeground();
        if (uploads.busy()) {
            awake.acquire(AWAKE_MILLISECONDS);
        } else {
            finish();
        }
        return START_STICKY;
    }

    /** Android 15 calls this when the day's allowance of syncing in the foreground is used up. */
    @Override
    public void onTimeout(int startId, int fgsType) {
        uploads.giveUp();
        finish();
    }

    @Override
    public void onDestroy() {
        uploads.unlisten(this);
        main.removeCallbacksAndMessages(null);
        letSleep();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @Override
    public void idle() {
        main.post(() -> {
            // An upload sent since has the service stay.
            if (!uploads.busy()) {
                finish();
            }
        });
    }

    @Override
    public void progress(String id, long sent, long total) {}

    @Override
    public void settled(UploadRecord upload) {}

    private void enterForeground() {
        try {
            ServiceCompat.startForeground(
                this,
                UploadNotices.SENDING_ID,
                UploadNotices.sending(this, uploads.sendingTitle()),
                ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
            );
        } catch (RuntimeException e) {
            // The system refuses a service started again in the background from Android 12
            // (`ForegroundServiceStartNotAllowedException`), and one whose allowance is used
            // up; the uploads go on for as long as the process is left alone.
            Log.w(TAG, "uploads go on without a foreground service", e);
        }
    }

    private void finish() {
        letSleep();
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE);
        stopSelf();
    }

    private void letSleep() {
        if (awake.isHeld()) {
            awake.release();
        }
    }
}
