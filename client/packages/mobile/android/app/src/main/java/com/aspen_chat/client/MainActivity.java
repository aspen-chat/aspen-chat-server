package com.aspen_chat.client;

import android.os.Bundle;
import com.getcapacitor.BridgeActivity;
import com.getcapacitor.Plugin;
import com.aspen_chat.client.age.AspenAgePlugin;
import com.aspen_chat.client.files.AspenFilesPlugin;
import com.aspen_chat.client.input.AspenKeyboardContentPlugin;
import com.aspen_chat.client.media.AspenMediaPlugin;
import com.aspen_chat.client.navigation.AspenNavigationPlugin;
import com.aspen_chat.client.passkeys.AspenPasskeysPlugin;
import com.aspen_chat.client.push.AspenPushPlugin;
import com.aspen_chat.client.secrets.AspenSecretsPlugin;
import com.aspen_chat.client.uploads.AspenUploadsPlugin;

public class MainActivity extends BridgeActivity {

    @Override
    public void onCreate(Bundle savedInstanceState) {
        // The app's own plugins, which Capacitor does not find among the npm packages.
        registerPlugin(AspenPushPlugin.class);
        registerPlugin(AspenSecretsPlugin.class);
        registerPlugin(AspenFilesPlugin.class);
        registerPlugin(AspenNavigationPlugin.class);
        registerPlugin(AspenPasskeysPlugin.class);
        registerPlugin(AspenAgePlugin.class);
        registerPlugin(AspenUploadsPlugin.class);
        registerPlugin(AspenKeyboardContentPlugin.class);
        registerPlugin(AspenMediaPlugin.class);
        registerCallPlugin();
        super.onCreate(savedInstanceState);
    }

    /**
     * The call engine's plugin, which is in the build only with the call layer
     * (`app/build.gradle` leaves `call/` out without WebRTC and the engine's bindings), so it is
     * named rather than referenced: a build without it has no plugin, and the web view's calls
     * run in the page as they do in a browser.
     */
    private void registerCallPlugin() {
        try {
            registerPlugin(Class.forName(CALL_PLUGIN).asSubclass(Plugin.class));
        } catch (ClassNotFoundException absent) {
            // The build without the call layer.
        }
    }

    private static final String CALL_PLUGIN = "com.aspen_chat.client.call.AspenCallPlugin";
}
