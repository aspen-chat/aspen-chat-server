package com.aspen_chat.client.push;

import android.content.Context;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * A new session token for one of the accounts the app keeps for push (spec/push.md, The app),
 * from its refresh token, for whatever acts as that account while the web view is not there to
 * renew it: the push handler reading a message, and the call engine joining a call.
 */
public final class SessionRefresh {

    private static final int TIMEOUT_MILLISECONDS = 8000;

    private SessionRefresh() {}

    /**
     * Gets a session token with the account's refresh token; {@code null} when the deployment
     * refuses, which means the account can no longer act. The caller keeps what it gets
     * ({@link PushStore#setSessionToken}).
     */
    public static String refresh(Context context, JSONObject account) throws IOException, JSONException {
        HttpURLConnection connection = open(context, account.getString("origin") + "/api/v1/auth/token-refresh");
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

    /** Opens a request that asks for its answer in the languages the phone shows the app in. */
    public static HttpURLConnection open(Context context, String url) throws IOException {
        HttpURLConnection connection = (HttpURLConnection) new URL(url).openConnection();
        connection.setConnectTimeout(TIMEOUT_MILLISECONDS);
        connection.setReadTimeout(TIMEOUT_MILLISECONDS);
        connection.setRequestProperty("Accept", "application/json");
        String languages = context.getResources().getConfiguration().getLocales().toLanguageTags();
        if (!languages.isEmpty()) {
            connection.setRequestProperty("Accept-Language", languages);
        }
        return connection;
    }

    static String body(InputStream in) throws IOException {
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
}
