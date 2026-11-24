package com.aspen_chat.client.push;

import android.content.Context;
import com.aspen_chat.client.secrets.SecretStore;
import java.util.function.Predicate;
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

    /** Forgets the state, as before the app wrote one. */
    public synchronized void remove() {
        store.remove(KEY);
    }

    /**
     * The account a push's subscription names; {@code null} when none does, or when the state is
     * of a version this code does not know, whose fields may mean something else.
     */
    public synchronized JSONObject account(String subscription) {
        return accountWhere(account -> subscription.equals(account.optString("subscription")));
    }

    /**
     * The first account {@code wanted} accepts; {@code null} when none does, or as {@link #account}
     * is.
     */
    public synchronized JSONObject accountWhere(Predicate<JSONObject> wanted) {
        JSONArray accounts = accounts();
        if (accounts == null) {
            return null;
        }
        try {
            for (int i = 0; i < accounts.length(); i++) {
                JSONObject account = accounts.getJSONObject(i);
                if (wanted.test(account)) {
                    return account;
                }
            }
        } catch (JSONException e) {
            return null;
        }
        return null;
    }

    /** The accounts kept; {@code null} when there is no state, or one of a version this code does not know. */
    private JSONArray accounts() {
        String state = load();
        if (state == null) {
            return null;
        }
        try {
            JSONObject parsed = new JSONObject(state);
            return parsed.optInt("version") == VERSION ? parsed.getJSONArray("accounts") : null;
        } catch (JSONException e) {
            return null;
        }
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
