package com.aspen_chat.client;

import android.os.Bundle;
import com.getcapacitor.BridgeActivity;
import com.aspen_chat.client.age.AspenAgePlugin;
import com.aspen_chat.client.audio.AspenAudioRoutePlugin;
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
        registerPlugin(AspenAudioRoutePlugin.class);
        registerPlugin(AspenUploadsPlugin.class);
        registerPlugin(AspenKeyboardContentPlugin.class);
        registerPlugin(AspenMediaPlugin.class);
        super.onCreate(savedInstanceState);
    }
}
