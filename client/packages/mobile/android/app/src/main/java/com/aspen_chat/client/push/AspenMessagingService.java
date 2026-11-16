package com.aspen_chat.client.push;

import androidx.annotation.NonNull;
import com.capacitorjs.plugins.pushnotifications.MessagingService;
import com.google.firebase.messaging.RemoteMessage;
import java.util.Map;

/**
 * Receives every FCM message for the app, in place of the push plugin's own service (the manifest
 * removes that one), which it extends so the plugin still learns of new tokens. A push from the
 * relay (spec/push.md) is decrypted and shown here, whether or not the app is open; anything
 * else goes to the plugin as before. A call comes marked {@code k: call} beside the ciphertext
 * (spec/push.md, Calls), a hint the relay adds for a messaging service that would prepare before
 * decrypting; this one decrypts at once on every push, so the pointer's kind alone decides.
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
