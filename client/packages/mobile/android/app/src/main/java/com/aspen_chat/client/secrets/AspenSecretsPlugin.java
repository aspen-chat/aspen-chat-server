package com.aspen_chat.client.secrets;

import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;

/** The app's saved sign-ins (`AspenSecrets` in the app's `src/api/sessions.ts`). */
@CapacitorPlugin(name = "AspenSecrets")
public class AspenSecretsPlugin extends Plugin {

    private static final String FILE = "aspen_sessions";
    private static final String KEY = "sessions";

    private SecretStore store() {
        return new SecretStore(getContext(), FILE);
    }

    @PluginMethod
    public void read(PluginCall call) {
        SecretStore.Read read = store().read(KEY);
        JSObject result = new JSObject();
        if (read.failed) {
            result.put("status", "unreadable");
        } else {
            result.put("status", "ok");
            result.put("contents", read.text == null ? JSObject.NULL : read.text);
        }
        call.resolve(result);
    }

    @PluginMethod
    public void write(PluginCall call) {
        String contents = call.getString("contents");
        if (contents == null) {
            call.reject("contents is required");
            return;
        }
        JSObject result = new JSObject();
        result.put("kept", store().write(KEY, contents));
        call.resolve(result);
    }
}
