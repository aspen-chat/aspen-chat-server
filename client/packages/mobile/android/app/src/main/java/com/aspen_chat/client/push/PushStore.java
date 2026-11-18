package com.aspen_chat.client.push;

import android.content.Context;
import com.aspen_chat.client.secrets.SecretStore;
import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

/**
 * The {@code PushState} the app keeps for its notification code (spec/push.md, The app), which
 * holds sessions and the subscriptions' private keys, sealed by {@link SecretStore}.
 */
public final class PushStore {

    private static final String FILE = "aspen_push";
    private static final String KEY = "state";
    /** The {@code version} of the state this code reads. */
    private static final int VERSION = 1;

    private final SecretStore store;

    public PushStore(Context context) {
        store = new SecretStore(context, FILE);
    }

    /** The state as the app wrote it; {@code null} before it has, or when it cannot be opened. */
    public synchronized String load() {
        return store.load(KEY);
    }

    public synchronized void save(String state) {
        store.write(KEY, state);
    }

    /**
     * The account a push's subscription names; {@code null} when none does, or when the state is
     * of a version this code does not know, whose fields may mean something else.
     */
    public synchronized JSONObject account(String subscription) {
        String state = load();
        if (state == null) {
            return null;
        }
        try {
            JSONObject parsed = new JSONObject(state);
            if (parsed.optInt("version") != VERSION) {
                return null;
            }
            JSONArray accounts = parsed.getJSONArray("accounts");
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
