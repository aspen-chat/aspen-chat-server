package com.aspen_chat.client;

import android.os.Bundle;
import com.getcapacitor.BridgeActivity;
import com.aspen_chat.client.age.AspenAgePlugin;
import com.aspen_chat.client.files.AspenFilesPlugin;
import com.aspen_chat.client.navigation.AspenNavigationPlugin;
import com.aspen_chat.client.passkeys.AspenPasskeysPlugin;
import com.aspen_chat.client.push.AspenPushPlugin;

public class MainActivity extends BridgeActivity {

    @Override
    public void onCreate(Bundle savedInstanceState) {
        // The app's own plugins, which Capacitor does not find among the npm packages.
        registerPlugin(AspenPushPlugin.class);
        registerPlugin(AspenFilesPlugin.class);
        registerPlugin(AspenNavigationPlugin.class);
        registerPlugin(AspenPasskeysPlugin.class);
        registerPlugin(AspenAgePlugin.class);
        super.onCreate(savedInstanceState);
    }
}
