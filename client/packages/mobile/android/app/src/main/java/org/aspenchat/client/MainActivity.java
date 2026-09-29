package org.aspenchat.client;

import android.os.Bundle;
import com.getcapacitor.BridgeActivity;
import org.aspenchat.client.push.AspenPushPlugin;

public class MainActivity extends BridgeActivity {

    @Override
    public void onCreate(Bundle savedInstanceState) {
        // The app's own plugins, which Capacitor does not find among the npm packages.
        registerPlugin(AspenPushPlugin.class);
        super.onCreate(savedInstanceState);
    }
}
