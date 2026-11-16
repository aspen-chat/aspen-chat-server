package com.aspen_chat.client;

import android.content.res.Configuration;
import android.os.Bundle;
import com.getcapacitor.BridgeActivity;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginHandle;
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

    /** The call plugin, where the build has it, as what it hears of the picture in picture. */
    private CallPictureInPictureHost pictureInPictureHost() {
        PluginHandle handle = getBridge() == null ? null : getBridge().getPlugin("AspenCall");
        Plugin instance = handle == null ? null : handle.getInstance();
        return instance instanceof CallPictureInPictureHost ? (CallPictureInPictureHost) instance : null;
    }

    @Override
    protected void onUserLeaveHint() {
        super.onUserLeaveHint();
        CallPictureInPictureHost host = pictureInPictureHost();
        if (host != null) {
            host.onUserLeaveHint();
        }
    }

    @Override
    public void onPictureInPictureModeChanged(boolean isInPictureInPictureMode, Configuration newConfig) {
        super.onPictureInPictureModeChanged(isInPictureInPictureMode, newConfig);
        CallPictureInPictureHost host = pictureInPictureHost();
        if (host != null) {
            host.onPictureInPictureModeChanged(isInPictureInPictureMode);
        }
    }
}
