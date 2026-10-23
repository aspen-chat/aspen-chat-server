package com.aspen_chat.client.passkeys;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import androidx.credentials.CredentialManager;
import androidx.credentials.CredentialManagerCallback;
import androidx.credentials.GetCredentialResponse;
import androidx.credentials.exceptions.GetCredentialException;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import com.aspen_chat.client.MainActivity;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;
import org.junit.Test;
import org.junit.runner.RunWith;

/**
 * What Credential Manager does on a device for a deployment that does not name the app: the
 * failure must read as `unavailable`, so the app hands the ceremony to the system browser rather
 * than reporting it failed. On a device with no passkey provider it fails the same way.
 */
@RunWith(AndroidJUnit4.class)
public class PasskeyPlatformTest {

    /** Options for a domain whose Digital Asset Links name no Aspen app. */
    private static final String OPTIONS =
            "{\"challenge\":\"AAECAwQFBgcICQoLDA0ODw\",\"rpId\":\"example.com\","
                    + "\"allowCredentials\":[],\"userVerification\":\"required\",\"timeout\":30000}";

    @Test
    public void aDeploymentThatDoesNotNameTheAppSendsItToTheBrowser() throws InterruptedException {
        CountDownLatch done = new CountDownLatch(1);
        AtomicReference<Exception> failed = new AtomicReference<>();
        try (ActivityScenario<MainActivity> scenario = ActivityScenario.launch(MainActivity.class)) {
            scenario.onActivity(
                    activity ->
                            CredentialManager.create(activity)
                                    .getCredentialAsync(
                                            activity,
                                            AspenPasskeysPlugin.getRequest(OPTIONS),
                                            null,
                                            activity.getMainExecutor(),
                                            new CredentialManagerCallback<
                                                    GetCredentialResponse, GetCredentialException>() {
                                                @Override
                                                public void onResult(GetCredentialResponse response) {
                                                    done.countDown();
                                                }

                                                @Override
                                                public void onError(GetCredentialException e) {
                                                    failed.set(e);
                                                    done.countDown();
                                                }
                                            }));
            assertTrue("Credential Manager answered", done.await(30, TimeUnit.SECONDS));
        }
        assertNotNull("no credential for a domain that names no app", failed.get());
        assertEquals(
                failed.get().toString(), PasskeyFailure.UNAVAILABLE, PasskeyFailure.of(failed.get()));
    }
}
