package org.aspenchat.client.push;

import androidx.annotation.NonNull;
import com.capacitorjs.plugins.pushnotifications.MessagingService;
import com.google.firebase.messaging.RemoteMessage;
import java.util.Map;

/**
 * Receives every FCM message for the app, in place of the push plugin's own service (the manifest
 * removes that one), which it extends so the plugin still learns of new tokens. A push from the
 * relay (spec/push.md) is decrypted and shown here, whether or not the app is open; anything
 * else goes to the plugin as before.
 */
public class AspenMessagingService extends MessagingService {

    @Override
    public void onMessageReceived(@NonNull RemoteMessage remoteMessage) {
        Map<String, String> data = remoteMessage.getData();
        String subscription = data.get("s");
        String ciphertext = data.get("c");
        if (subscription != null && ciphertext != null) {
            // Called off the main thread, with time enough to fetch and show.
            new PushHandler(this).handle(subscription, ciphertext);
            return;
        }
        super.onMessageReceived(remoteMessage);
    }
}
