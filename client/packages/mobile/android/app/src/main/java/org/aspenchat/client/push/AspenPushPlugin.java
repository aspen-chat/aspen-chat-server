package org.aspenchat.client.push;

import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import com.google.firebase.FirebaseApp;
import org.aspenchat.client.R;

/**
 * The app's native side of push (`AspenPush` in the app's `src/api/pushBridge.ts`): which
 * platform, app, and relay this build is, and the `PushState` the notification code reads.
 */
@CapacitorPlugin(name = "AspenPush")
public class AspenPushPlugin extends Plugin {

    @PluginMethod
    public void describe(PluginCall call) {
        String relay = getContext().getString(R.string.aspen_push_relay);
        if (relay.isEmpty()) {
            call.reject("this build names no push relay");
            return;
        }
        String project;
        try {
            project = FirebaseApp.getInstance().getOptions().getProjectId();
        } catch (IllegalStateException e) {
            call.reject("this build has no Firebase project (google-services.json)");
            return;
        }
        if (project == null) {
            call.reject("this build's Firebase project has no id");
            return;
        }
        JSObject result = new JSObject();
        result.put("platform", "fcm");
        result.put("app", project);
        result.put("environment", "production");
        result.put("relay", relay);
        call.resolve(result);
    }

    @PluginMethod
    public void loadState(PluginCall call) {
        JSObject result = new JSObject();
        String state = new PushStore(getContext()).load();
        result.put("state", state == null ? JSObject.NULL : state);
        call.resolve(result);
    }

    @PluginMethod
    public void saveState(PluginCall call) {
        String state = call.getString("state");
        if (state == null) {
            call.reject("state is required");
            return;
        }
        new PushStore(getContext()).save(state);
        call.resolve();
    }
}
