package com.aspen_chat.client.uploads;

import org.json.JSONException;
import org.json.JSONObject;

/**
 * One upload as the app keeps it between `send` and `discard` (`Uploads`): what `send` was given,
 * and how the upload stands. It holds the account's session and refresh tokens, so it is kept
 * only in the app's private storage.
 */
public final class UploadRecord {

    public static final String SENDING = "sending";
    public static final String DONE = "done";
    public static final String FAILED = "failed";

    public static final String REFUSED = "refused";
    public static final String UNREACHABLE = "unreachable";

    /** The attachment's id. */
    public String id;
    public String origin;
    public String userId;
    public String sessionToken;
    public String refreshToken;
    public String uploadUrl;
    /** The `Content-Type` `uploadUrl` is signed for. */
    public String contentType;
    public String name;
    public String mimeType;
    public long byteSize;
    /**
     * The languages the deployment is asked to answer in, an `Accept-Language` header; empty
     * to leave it unsaid.
     */
    public String acceptLanguage = "";
    public String failedTitle;
    public String failedBody;
    public String sendingTitle;

    /** {@link #SENDING}, {@link #DONE}, or {@link #FAILED}. */
    public String state = SENDING;
    /** How many bytes the request to storage under way has written, or the last one wrote. */
    public long sent;
    /** Once done: the confirmed attachment record, JSON text as the deployment answered. */
    public String attachment;
    /** Once failed: {@link #REFUSED} or {@link #UNREACHABLE}. */
    public String cause;
    /** For a refusal, the deployment's own words; {@code null} when it gave none. */
    public String reason;
    /** When `send` was last called for it, in milliseconds since the epoch. */
    public long startedAt;

    public UploadRecord copy() {
        UploadRecord copy = new UploadRecord();
        copy.id = id;
        copy.origin = origin;
        copy.userId = userId;
        copy.sessionToken = sessionToken;
        copy.refreshToken = refreshToken;
        copy.uploadUrl = uploadUrl;
        copy.contentType = contentType;
        copy.name = name;
        copy.mimeType = mimeType;
        copy.byteSize = byteSize;
        copy.acceptLanguage = acceptLanguage;
        copy.failedTitle = failedTitle;
        copy.failedBody = failedBody;
        copy.sendingTitle = sendingTitle;
        copy.state = state;
        copy.sent = sent;
        copy.attachment = attachment;
        copy.cause = cause;
        copy.reason = reason;
        copy.startedAt = startedAt;
        return copy;
    }

    /** The upload as the page reads it: `NativeUpload` in the app's `src/api/uploadsBridge.ts`. */
    public JSONObject toNative() {
        try {
            JSONObject upload = new JSONObject()
                .put("id", id)
                .put("name", name)
                .put("mimeType", mimeType)
                .put("byteSize", byteSize)
                .put("state", state)
                .put("sent", sent);
            if (DONE.equals(state)) {
                upload.put("attachment", attachment);
            }
            if (FAILED.equals(state)) {
                upload.put("cause", cause);
                upload.put("reason", reason == null ? JSONObject.NULL : reason);
            }
            return upload;
        } catch (JSONException e) {
            // Only a non-finite number makes `put` throw, and none is put.
            throw new IllegalStateException(e);
        }
    }

    JSONObject toJson() throws JSONException {
        return new JSONObject()
            .put("id", id)
            .put("origin", origin)
            .put("userId", userId)
            .put("sessionToken", sessionToken)
            .put("refreshToken", refreshToken)
            .put("uploadUrl", uploadUrl)
            .put("contentType", contentType)
            .put("name", name)
            .put("mimeType", mimeType)
            .put("byteSize", byteSize)
            .put("acceptLanguage", acceptLanguage)
            .put("failedTitle", failedTitle)
            .put("failedBody", failedBody)
            .put("sendingTitle", sendingTitle)
            .put("state", state)
            .put("sent", sent)
            .put("attachment", attachment)
            .put("cause", cause)
            .put("reason", reason)
            .put("startedAt", startedAt);
    }

    static UploadRecord fromJson(JSONObject json) throws JSONException {
        UploadRecord record = new UploadRecord();
        record.id = json.getString("id");
        record.origin = json.getString("origin");
        record.userId = json.getString("userId");
        record.sessionToken = json.getString("sessionToken");
        record.refreshToken = json.getString("refreshToken");
        record.uploadUrl = json.getString("uploadUrl");
        record.contentType = json.getString("contentType");
        record.name = json.getString("name");
        record.mimeType = json.getString("mimeType");
        record.byteSize = json.getLong("byteSize");
        record.acceptLanguage = json.optString("acceptLanguage");
        record.failedTitle = json.optString("failedTitle");
        record.failedBody = json.optString("failedBody");
        record.sendingTitle = json.optString("sendingTitle");
        record.state = json.getString("state");
        record.sent = json.optLong("sent");
        record.attachment = text(json, "attachment");
        record.cause = text(json, "cause");
        record.reason = text(json, "reason");
        record.startedAt = json.optLong("startedAt");
        return record;
    }

    /** The text at `key`; {@code null} when it is absent, JSON's null, or not text. */
    static String text(JSONObject json, String key) {
        Object value = json.opt(key);
        return value instanceof String ? (String) value : null;
    }
}
