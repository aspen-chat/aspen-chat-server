package com.aspen_chat.client.uploads;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.os.Build;
import android.service.notification.StatusBarNotification;
import androidx.core.app.NotificationCompat;
import androidx.core.app.NotificationManagerCompat;
import com.aspen_chat.client.R;

/**
 * The notifications uploads show: the one that stands while files are sent
 * ({@link UploadService}), and one for each upload that fails while the app is not in front,
 * since no page is then showing that it did.
 */
final class UploadNotices implements Uploads.Listener {

    static final int SENDING_ID = 7301;
    /** Every failure's notification has this id, and its upload's id as its tag. */
    private static final int FAILED_ID = 7302;
    private static final String SENDING_CHANNEL = "uploads";
    private static final String FAILED_CHANNEL = "upload_failures";

    /** Whether the app's activity is resumed, as `AspenUploadsPlugin` last heard. */
    static volatile boolean inFront;

    private final Context context;

    UploadNotices(Context context) {
        this.context = context.getApplicationContext();
    }

    /** The notification that stands while files are sent, saying `title` (the app's own words for none). */
    static Notification sending(Context context, String title) {
        String name = context.getString(R.string.aspen_uploads_channel_name);
        channel(context, SENDING_CHANNEL, name, NotificationManager.IMPORTANCE_LOW);
        return new NotificationCompat.Builder(context, SENDING_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_aspen)
            .setColor(0xFF047857)
            .setContentTitle(title == null || title.isEmpty() ? name : title)
            .setProgress(0, 0, true)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(NotificationCompat.CATEGORY_PROGRESS)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setContentIntent(opensApp(context, 0))
            .build();
    }

    /** Takes down what was shown of upload `id` failing: it was sent again, or let go. */
    static void clear(Context context, String id) {
        context.getSystemService(NotificationManager.class).cancel(id, FAILED_ID);
    }

    /** Takes down every failure shown: the app is in front, and its page shows them itself. */
    static void clearAll(Context context) {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        for (StatusBarNotification shown : manager.getActiveNotifications()) {
            if (shown.getId() == FAILED_ID && shown.getTag() != null) {
                manager.cancel(shown.getTag(), FAILED_ID);
            }
        }
    }

    @Override
    public void settled(UploadRecord upload) {
        // Without leave to post notifications the failure waits for the app to be opened.
        if (
            !UploadRecord.FAILED.equals(upload.state) ||
            inFront ||
            !NotificationManagerCompat.from(context).areNotificationsEnabled()
        ) {
            return;
        }
        String name = context.getString(R.string.aspen_upload_failures_channel_name);
        channel(context, FAILED_CHANNEL, name, NotificationManager.IMPORTANCE_DEFAULT);
        PendingIntent tap = opensApp(context, upload.id.hashCode());
        if (tap == null) {
            return;
        }
        String body = upload.failedBody.isEmpty() ? upload.name : upload.failedBody;
        NotificationCompat.Builder builder = new NotificationCompat.Builder(context, FAILED_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_aspen)
            .setColor(0xFF047857)
            .setContentTitle(upload.failedTitle.isEmpty() ? name : upload.failedTitle)
            .setContentText(body)
            .setStyle(new NotificationCompat.BigTextStyle().bigText(body))
            .setCategory(NotificationCompat.CATEGORY_ERROR)
            .setAutoCancel(true)
            .setContentIntent(tap);
        context.getSystemService(NotificationManager.class).notify(upload.id, FAILED_ID, builder.build());
    }

    @Override
    public void progress(String id, long sent, long total) {}

    @Override
    public void idle() {}

    private static void channel(Context context, String id, String name, int importance) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            context
                .getSystemService(NotificationManager.class)
                .createNotificationChannel(new NotificationChannel(id, name, importance));
        }
    }

    /** A tap that brings the app to the front; {@code null} when it has no launcher activity. */
    private static PendingIntent opensApp(Context context, int request) {
        Intent open = context.getPackageManager().getLaunchIntentForPackage(context.getPackageName());
        if (open == null) {
            return null;
        }
        open.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        return PendingIntent.getActivity(
            context,
            request,
            open,
            PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE
        );
    }
}
