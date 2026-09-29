package org.aspenchat.client.push;

import android.content.Context;
import android.content.SharedPreferences;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * The {@code PushState} the app keeps for its notification code (spec/push.md, The app), in the
 * app's private storage, where only this app can read it.
 */
public final class PushStore {

    private static final String FILE = "aspen_push";
    private static final String KEY = "state";

    private final SharedPreferences preferences;

    public PushStore(Context context) {
        preferences = context.getApplicationContext().getSharedPreferences(FILE, Context.MODE_PRIVATE);
    }

    /** The state as the app wrote it; {@code null} before it has. */
    public synchronized String load() {
        return preferences.getString(KEY, null);
    }

    public synchronized void save(String state) {
        preferences.edit().putString(KEY, state).apply();
    }

    /** The account a push's subscription names; {@code null} when none does. */
    public synchronized JSONObject account(String subscription) {
        String state = load();
        if (state == null) {
            return null;
        }
        try {
            JSONArray accounts = new JSONObject(state).getJSONArray("accounts");
            for (int i = 0; i < accounts.length(); i++) {
                JSONObject account = accounts.getJSONObject(i);
                if (subscription.equals(account.optString("subscription"))) {
                    return account;
                }
            }
        } catch (JSONException e) {
            return null;
        }
        return null;
    }

    /** Keeps a session token the notification code got, so the next push need not get another. */
    public synchronized void setSessionToken(String subscription, String sessionToken) {
        String state = load();
        if (state == null) {
            return;
        }
        try {
            JSONObject parsed = new JSONObject(state);
            JSONArray accounts = parsed.getJSONArray("accounts");
            for (int i = 0; i < accounts.length(); i++) {
                JSONObject account = accounts.getJSONObject(i);
                if (subscription.equals(account.optString("subscription"))) {
                    account.put("sessionToken", sessionToken);
                }
            }
            save(parsed.toString());
        } catch (JSONException ignored) {
            // A state the app wrote badly is the app's to rewrite; this token is simply not kept.
        }
    }
}
