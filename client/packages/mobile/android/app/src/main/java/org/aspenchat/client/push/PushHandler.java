package org.aspenchat.client.push;

import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.os.Build;
import android.service.notification.StatusBarNotification;
import android.util.Base64;
import android.util.Log;
import androidx.core.app.NotificationCompat;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import org.aspenchat.client.R;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * What the phone does with a push while the app may be closed (spec/push.md, The app): finds the
 * account its subscription names, decrypts the pointer, and shows the message it points to, or
 * takes down what was shown for a channel read elsewhere or a message deleted.
 */
public final class PushHandler {

    private static final String TAG = "AspenPush";
    private static final String CHANNEL = "messages";
    private static final int TIMEOUT_MILLISECONDS = 8000;
    private static final Pattern USER_TAG = Pattern.compile("<@([0-9a-fA-F-]{36})>");
    private static final Pattern ROLE_TAG = Pattern.compile("<@&[0-9a-fA-F-]{36}>");

    private final Context context;
    private final PushStore store;

    public PushHandler(Context context) {
        this.context = context.getApplicationContext();
        this.store = new PushStore(context);
    }

    /** Handles one push's {@code s} and {@code c}; anything that fails shows nothing. */
    public void handle(String subscription, String ciphertext) {
        try {
            JSONObject account = store.account(subscription);
            if (account == null) {
                return;
            }
            JSONObject keys = account.getJSONObject("keys");
            byte[] plaintext = WebPush.decrypt(
                decode(ciphertext),
                decode(keys.getJSONObject("privateKey").getString("d")),
                decode(keys.getString("publicKey")),
                decode(keys.getString("auth"))
            );
            JSONObject pointer = new JSONObject(new String(plaintext, StandardCharsets.UTF_8));
            if (pointer.optInt("v") != 1) {
                return;
            }
            String kind = pointer.optString("kind");
            String channel = pointer.getString("channel");
            String message = pointer.getString("message");
            switch (kind) {
                case "message":
                    show(subscription, account, channel, message);
                    break;
                case "read":
                    takeDown(channel, message, true);
                    break;
                case "deleted":
                    takeDown(channel, message, false);
                    break;
                default:
                    // A kind this app does not know yet.
                    break;
            }
        } catch (Exception e) {
            Log.w(TAG, "a push could not be shown", e);
        }
    }

    private void show(String subscription, JSONObject account, String channel, String messageId)
        throws IOException, JSONException {
        String origin = account.getString("origin");
        JSONObject read = fetch(subscription, account, origin + "/api/v1/messages/" + messageId + "?include=authors,channels,mentions");
        if (read == null) {
            return;
        }
        JSONObject message = read.getJSONObject("data");
        JSONObject included = read.optJSONObject("included");
        JSONObject author = find(included, "users", message.getString("author"));
        JSONObject posted = find(included, "channels", message.getString("channelId"));
        String name = nameOf(author);
        String where = null;
        String community = null;
        String parentChannel = null;
        if (posted != null) {
            community = posted.isNull("community") ? null : posted.optString("community");
            parentChannel = posted.isNull("parentChannel") ? null : posted.optString("parentChannel");
            String type = posted.optString("ty");
            if (!"dm".equals(type) && !"groupDm".equals(type) && !posted.optString("name").isEmpty()) {
                where = "#" + posted.optString("name");
            }
        }
        String text = readable(message.optString("content"), included);
        if (text.isEmpty()) {
            text = context.getString(R.string.aspen_push_new_message);
        }

        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            manager.createNotificationChannel(
                new NotificationChannel(
                    CHANNEL,
                    context.getString(R.string.aspen_push_channel_name),
                    NotificationManager.IMPORTANCE_HIGH
                )
            );
        }
        Intent open = context.getPackageManager().getLaunchIntentForPackage(context.getPackageName());
        if (open == null) {
            return;
        }
        open.addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP | Intent.FLAG_ACTIVITY_CLEAR_TOP);
        // The push plugin passes a tapped notification's extras to the app when they carry a
        // message id; the rest say where the message is (spec/push.md, The app).
        open.putExtra("google.message_id", messageId);
        open.putExtra("origin", origin);
        open.putExtra("channel", message.getString("channelId"));
        open.putExtra("message", messageId);
        open.putExtra("community", community);
        open.putExtra("parentChannel", parentChannel);
        PendingIntent tap = PendingIntent.getActivity(
            context,
            messageId.hashCode(),
            open,
            PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE
        );
        NotificationCompat.Builder builder = new NotificationCompat.Builder(context, CHANNEL)
            // The status bar draws a small icon by its alpha alone; the colour tints it where shown.
            .setSmallIcon(R.drawable.ic_stat_aspen)
            .setColor(0xFF047857)
            .setContentTitle(where == null ? name : name + " · " + where)
            .setContentText(text)
            .setStyle(new NotificationCompat.BigTextStyle().bigText(text))
            .setGroup(channel)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(tap);
        manager.notify(tagOf(channel, messageId), 0, builder.build());
    }

    /**
     * Takes down what was shown for {@code channel}: everything up to {@code message} when it was
     * read, or that one message when it was deleted.
     */
    private void takeDown(String channel, String message, boolean upTo) {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        String prefix = channel + "/";
        for (StatusBarNotification shown : manager.getActiveNotifications()) {
            String tag = shown.getTag();
            if (tag == null || !tag.startsWith(prefix)) {
                continue;
            }
            String id = tag.substring(prefix.length());
            // Message ids are UUIDv7, so an earlier message sorts first.
            if (upTo ? id.compareTo(message) <= 0 : id.equals(message)) {
                manager.cancel(tag, shown.getId());
            }
        }
    }

    /** What the app calls a user: their display name, or else their username. */
    private static String nameOf(JSONObject user) {
        if (user == null) {
            return "";
        }
        String display = user.isNull("displayName") ? "" : user.optString("displayName");
        return display.isEmpty() ? user.optString("name") : display;
    }

    private static String tagOf(String channel, String message) {
        return channel + "/" + message;
    }

    /** The message's text as a notification shows it: tags as names, not tokens. */
    private String readable(String content, JSONObject included) {
        Matcher matcher = USER_TAG.matcher(content);
        StringBuffer out = new StringBuffer();
        while (matcher.find()) {
            JSONObject user = find(included, "users", matcher.group(1));
            String name = user == null ? "…" : nameOf(user);
            matcher.appendReplacement(out, Matcher.quoteReplacement("@" + name));
        }
        matcher.appendTail(out);
        return ROLE_TAG.matcher(out.toString()).replaceAll("@…").trim();
    }

    private static JSONObject find(JSONObject included, String type, String id) {
        if (included == null) {
            return null;
        }
        JSONArray records = included.optJSONArray(type);
        if (records == null) {
            return null;
        }
        for (int i = 0; i < records.length(); i++) {
            JSONObject record = records.optJSONObject(i);
            if (record != null && id.equals(record.optString("id"))) {
                return record;
            }
        }
        return null;
    }

    /**
     * GETs {@code url} with the account's session, getting a new session token with its refresh
     * token when the one kept has expired; {@code null} when the account can no longer read.
     */
    private JSONObject fetch(String subscription, JSONObject account, String url) throws IOException, JSONException {
        String token = account.getString("sessionToken");
        for (int attempt = 0; attempt < 2; attempt++) {
            HttpURLConnection connection = open(url);
            connection.setRequestProperty("Authorization", "Bearer " + token);
            int status = connection.getResponseCode();
            if (status == 200) {
                return new JSONObject(body(connection.getInputStream()));
            }
            connection.disconnect();
            if (status != 401 || attempt > 0) {
                return null;
            }
            token = refresh(account);
            if (token == null) {
                return null;
            }
            store.setSessionToken(subscription, token);
        }
        return null;
    }

    private String refresh(JSONObject account) throws IOException, JSONException {
        HttpURLConnection connection = open(account.getString("origin") + "/api/v1/auth/token-refresh");
        connection.setRequestMethod("POST");
        connection.setDoOutput(true);
        connection.setRequestProperty("Content-Type", "application/json");
        JSONObject body = new JSONObject().put("refreshToken", account.getString("refreshToken"));
        try (OutputStream out = connection.getOutputStream()) {
            out.write(body.toString().getBytes(StandardCharsets.UTF_8));
        }
        if (connection.getResponseCode() != 200) {
            return null;
        }
        return new JSONObject(body(connection.getInputStream())).optString("sessionToken", null);
    }

    private static HttpURLConnection open(String url) throws IOException {
        HttpURLConnection connection = (HttpURLConnection) new URL(url).openConnection();
        connection.setConnectTimeout(TIMEOUT_MILLISECONDS);
        connection.setReadTimeout(TIMEOUT_MILLISECONDS);
        connection.setRequestProperty("Accept", "application/json");
        return connection;
    }

    private static String body(InputStream in) throws IOException {
        try (InputStream stream = in) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buffer = new byte[8192];
            int read;
            while ((read = stream.read(buffer)) != -1) {
                out.write(buffer, 0, read);
            }
            return out.toString("UTF-8");
        }
    }

    private static byte[] decode(String base64url) {
        return Base64.decode(base64url, Base64.URL_SAFE | Base64.NO_PADDING | Base64.NO_WRAP);
    }
}
