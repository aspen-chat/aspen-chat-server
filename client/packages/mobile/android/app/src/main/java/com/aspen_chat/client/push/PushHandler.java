package com.aspen_chat.client.push;

import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;
import android.service.notification.StatusBarNotification;
import android.util.Base64;
import android.util.Log;
import androidx.core.app.NotificationCompat;
import java.io.IOException;
import java.net.HttpURLConnection;
import java.nio.charset.StandardCharsets;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import com.aspen_chat.client.R;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * What the phone does with a push while the app may be closed (spec/push.md, The app): finds the
 * account its subscription names, decrypts the pointer, and shows the message or plugin's notice
 * it points to, takes down what was shown for a channel read elsewhere or a message deleted, or
 * hands a DM's call ringing the phone to the call layer.
 */
public final class PushHandler {

    private static final String TAG = "AspenPush";
    private static final String CHANNEL = "messages";
    /**
     * What a message is read with: who wrote it, where, and what its notification says, written
     * out by the deployment ({@code notificationTexts}), with the people it tags for a
     * deployment of a version that does not write it out.
     */
    private static final String MESSAGE_INCLUDE = "?include=authors,channels,mentions,memberships,notificationTexts";
    private static final Pattern USER_TAG = Pattern.compile("<@([0-9a-fA-F-]{36})>");
    private static final Pattern ROLE_TAG = Pattern.compile("<@&[0-9a-fA-F-]{36}>");
    /**
     * The call layer's entry for a ring (call/CallRings.kt), by name: a build without the layer
     * (app/build.gradle, {@code callLayer}) has no such class, and shows nothing for a call.
     */
    private static final String CALL_RINGS = "com.aspen_chat.client.call.CallRings";

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
            switch (kind) {
                case "message":
                    show(subscription, account, channel, pointer.getString("message"));
                    break;
                case "read":
                    takeDown(subscription, channel, pointer.getString("message"), true);
                    break;
                case "deleted":
                    takeDown(subscription, channel, pointer.getString("message"), false);
                    break;
                case "notice":
                    showNotice(subscription, account, channel, pointer.getString("notice"));
                    break;
                case "call":
                    ring(account, pointer);
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
        JSONObject read = fetch(subscription, account, origin + "/api/v1/messages/" + messageId + MESSAGE_INCLUDE);
        if (read == null) {
            return;
        }
        JSONObject message = read.getJSONObject("data");
        JSONObject included = read.optJSONObject("included");
        JSONObject author = find(included, "users", message.getString("author"));
        JSONObject posted = find(included, "channels", message.getString("channelId"));
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
        String nickname = nicknameOf(included, community, message.getString("author"));
        String name = nickname != null ? nickname : nameOf(author);
        String text = written(included, messageId);
        if (text == null) {
            String content = message.optString("content");
            // Text that may hold a spoiler is not shown (SpoilerMarkers): the message reads as
            // one with no text.
            text = SpoilerMarkers.present(content) ? "" : readable(content, included);
            if (text.trim().isEmpty()) {
                text = recorded(message);
            }
        }
        post(
            subscription,
            account.getString("userId"),
            origin,
            channel,
            messageId,
            message.getString("channelId"),
            messageId,
            community,
            parentChannel,
            where == null ? name : name + " · " + where,
            text
        );
    }

    /**
     * Hands a DM's call ringing the phone to the call layer, which shows it as an incoming call
     * within the five seconds a service started by a push has; nothing is fetched first, the
     * pointer carrying what the call shows.
     */
    private void ring(JSONObject account, JSONObject pointer) {
        try {
            Class<?> rings = Class.forName(CALL_RINGS);
            rings.getMethod("incoming", Context.class, JSONObject.class, JSONObject.class).invoke(null, context, account, pointer);
        } catch (ClassNotFoundException e) {
            Log.i(TAG, "a call rang the phone, but this build has no call layer to show it");
        } catch (ReflectiveOperationException e) {
            Log.w(TAG, "the call layer could not show a ring", e);
        }
    }

    /** Shows a plugin's notice: the plugin's name, and what it says. */
    private void showNotice(String subscription, JSONObject account, String channel, String noticeId)
        throws IOException, JSONException {
        String origin = account.getString("origin");
        JSONObject notice = fetch(subscription, account, origin + "/api/v1/users/@me/plugin-notices/" + noticeId);
        if (notice == null) {
            return;
        }
        post(
            subscription,
            account.getString("userId"),
            origin,
            channel,
            noticeId,
            notice.getString("channel"),
            notice.isNull("message") ? null : notice.optString("message"),
            notice.isNull("community") ? null : notice.optString("community"),
            notice.isNull("parentChannel") ? null : notice.optString("parentChannel"),
            notice.getString("title"),
            notice.getString("body")
        );
    }

    /**
     * Posts a notification tagged with its account's {@code subscription}, {@code channel}, and
     * {@code id} that opens {@code message} in {@code opens} (or the channel, for none) when
     * tapped, as {@code userId}. Accounts on one deployment share channels and messages, and each
     * has its own notification of them, which only its own reading takes down.
     */
    private void post(
        String subscription,
        String userId,
        String origin,
        String channel,
        String id,
        String opens,
        String message,
        String community,
        String parentChannel,
        String title,
        String text
    ) {
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
        // message id, which a notice's id stands in for; the rest say what to open
        // (spec/push.md, The app).
        open.putExtra("google.message_id", id);
        open.putExtra("origin", origin);
        open.putExtra("userId", userId);
        open.putExtra("channel", opens);
        open.putExtra("message", message);
        open.putExtra("community", community);
        open.putExtra("parentChannel", parentChannel);
        // Two pending intents are one unless their request codes or their intents differ, and
        // extras are not part of that comparison while data is. Hash codes can collide, so the
        // data names the notification, and a tap opens what its own notification showed. The
        // intent names the activity, so the address is never resolved, and the app opens only
        // addresses of its own link scheme.
        open.setData(
            new Uri.Builder()
                .scheme("aspen-push")
                .authority(channel)
                .appendPath(subscription)
                .appendPath(id)
                .build()
        );
        PendingIntent tap = PendingIntent.getActivity(
            context,
            id.hashCode(),
            open,
            PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE
        );
        NotificationCompat.Builder builder = new NotificationCompat.Builder(context, CHANNEL)
            // The status bar draws a small icon by its alpha alone; the colour tints it where shown.
            .setSmallIcon(R.drawable.ic_stat_aspen)
            .setColor(0xFF047857)
            .setContentTitle(title)
            .setContentText(text)
            .setStyle(new NotificationCompat.BigTextStyle().bigText(text))
            .setGroup(subscription + "/" + channel)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            .setContentIntent(tap);
        manager.notify(tagOf(subscription, channel, id), 0, builder.build());
    }

    /**
     * Takes down what was shown to {@code subscription}'s account for {@code channel}: everything up to {@code message} when it was
     * read, or that one message when it was deleted.
     */
    private void takeDown(String subscription, String channel, String message, boolean upTo) {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        String prefix = subscription + "/" + channel + "/";
        for (StatusBarNotification shown : manager.getActiveNotifications()) {
            String tag = shown.getTag();
            if (tag == null || !tag.startsWith(prefix)) {
                continue;
            }
            String id = tag.substring(prefix.length());
            // Message and notice ids are UUIDv7, so an earlier one sorts first: reading a channel
            // takes down the notices about it from before what was read.
            if (upTo ? id.compareTo(message) <= 0 : id.equals(message)) {
                manager.cancel(tag, shown.getId());
            }
        }
    }

    /**
     * What a message with no text of its own says, by its kind, when the deployment has not
     * said.
     */
    private String recorded(JSONObject message) {
        switch (message.optString("kind")) {
            case "poll":
                return context.getString(R.string.aspen_push_poll);
            case "pollClosed":
                return context.getString(R.string.aspen_push_poll_closed);
            case "memberJoined":
                return context.getString(R.string.aspen_push_member_joined);
            case "communityGrew": {
                String grew = context.getString(R.string.aspen_push_community_grew);
                JSONObject growth = message.optJSONObject("growth");
                if (growth == null || !growth.optBoolean("mentionEveryoneTaken")) {
                    return grew;
                }
                return grew + " " + context.getString(R.string.aspen_push_community_grew_mention_everyone);
            }
            default:
                return context.getString(R.string.aspen_push_new_message);
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

    /** The nickname {@code user} goes by in {@code community}, from the read's memberships, if any. */
    private static String nicknameOf(JSONObject included, String community, String user) {
        if (included == null || community == null) {
            return null;
        }
        JSONArray memberships = included.optJSONArray("userCommunities");
        if (memberships == null) {
            return null;
        }
        for (int i = 0; i < memberships.length(); i++) {
            JSONObject membership = memberships.optJSONObject(i);
            if (membership != null
                && community.equals(membership.optString("community"))
                && user.equals(membership.optString("user"))
                && !membership.isNull("nickname")
                && !membership.optString("nickname").isEmpty()) {
                return membership.optString("nickname");
            }
        }
        return null;
    }

    private static String tagOf(String subscription, String channel, String message) {
        return subscription + "/" + channel + "/" + message;
    }

    /**
     * What the deployment says the message's notification says, ready to show as given;
     * {@code null} from a deployment of a version that does not say.
     */
    private static String written(JSONObject included, String messageId) {
        JSONArray texts = included == null ? null : included.optJSONArray("notificationTexts");
        if (texts == null) {
            return null;
        }
        for (int i = 0; i < texts.length(); i++) {
            JSONObject text = texts.optJSONObject(i);
            if (text != null && messageId.equals(text.optString("message")) && !text.isNull("text")) {
                String given = text.optString("text");
                return given.isEmpty() ? null : given;
            }
        }
        return null;
    }

    /**
     * The message's text as a notification shows it when the deployment has not written it
     * out: tags as names, not tokens. Only for text that holds no spoiler's marker.
     */
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
            HttpURLConnection connection = SessionRefresh.open(context, url);
            connection.setRequestProperty("Authorization", "Bearer " + token);
            int status = connection.getResponseCode();
            if (status == 200) {
                return new JSONObject(SessionRefresh.body(connection.getInputStream()));
            }
            connection.disconnect();
            if (status != 401 || attempt > 0) {
                return null;
            }
            token = SessionRefresh.refresh(context, account);
            if (token == null) {
                return null;
            }
            store.setSessionToken(subscription, token);
        }
        return null;
    }

    private static byte[] decode(String base64url) {
        return Base64.decode(base64url, Base64.URL_SAFE | Base64.NO_PADDING | Base64.NO_WRAP);
    }
}
