package com.aspen_chat.client.passkeys;

import android.app.Activity;
import android.content.pm.PackageInfo;
import android.content.pm.PackageManager;
import android.content.pm.Signature;
import android.content.pm.SigningInfo;
import android.os.Build;
import androidx.credentials.CreateCredentialResponse;
import androidx.credentials.CreatePublicKeyCredentialRequest;
import androidx.credentials.CreatePublicKeyCredentialResponse;
import androidx.credentials.Credential;
import androidx.credentials.CredentialManager;
import androidx.credentials.CredentialManagerCallback;
import androidx.credentials.GetCredentialRequest;
import androidx.credentials.GetCredentialResponse;
import androidx.credentials.GetPublicKeyCredentialOption;
import androidx.credentials.PublicKeyCredential;
import androidx.credentials.exceptions.CreateCredentialException;
import androidx.credentials.exceptions.GetCredentialException;
import com.getcapacitor.JSArray;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.Collections;
import java.util.List;
import java.util.Locale;
import org.json.JSONException;

/**
 * Passkeys through Android's Credential Manager (`AspenPasskeys` in the app's
 * `src/features/auth/nativePasskeys.ts`), for deployments whose `assetlinks.json` names this app.
 * Android then lets the app use the deployment's passkeys as a page at its domain would, writing
 * the app's own origin (`android:apk-key-hash:` and its signing certificate's digest) into what
 * the authenticator signs, which the deployment accepts for the apps it names. Each call takes
 * the ceremony's `publicKey` options as the server gave them and answers the credential as the
 * JSON the server reads; a failure rejects with a code of `PasskeyFailure`.
 */
@CapacitorPlugin(name = "AspenPasskeys")
public class AspenPasskeysPlugin extends Plugin {

    /** Credential Manager offers passkeys from Android 9 on. */
    static final int FIRST_SDK = Build.VERSION_CODES.P;

    /**
     * This app's package and the SHA-256 digests of the certificates it is signed with, as
     * Digital Asset Links write them, for the page to find among those a deployment names;
     * `available` is false where Credential Manager offers no passkeys.
     */
    @PluginMethod
    public void identity(PluginCall call) {
        JSObject answer = new JSObject();
        answer.put("packageName", getContext().getPackageName());
        JSArray fingerprints = new JSArray();
        boolean available = Build.VERSION.SDK_INT >= FIRST_SDK;
        if (available) {
            for (Signature signature : signers()) {
                fingerprints.put(fingerprint(signature.toByteArray()));
            }
        }
        answer.put("available", available);
        answer.put("fingerprints", fingerprints);
        call.resolve(answer);
    }

    @PluginMethod
    public void create(PluginCall call) {
        Activity activity = getActivity();
        JSObject publicKey = call.getObject("publicKey");
        if (activity == null || publicKey == null || Build.VERSION.SDK_INT < FIRST_SDK) {
            call.reject("Credential Manager cannot run here", PasskeyFailure.UNAVAILABLE);
            return;
        }
        CredentialManager.create(activity)
                .createCredentialAsync(
                        activity,
                        createRequest(publicKey.toString()),
                        null,
                        activity.getMainExecutor(),
                        new CredentialManagerCallback<
                                CreateCredentialResponse, CreateCredentialException>() {
                            @Override
                            public void onResult(CreateCredentialResponse response) {
                                if (response instanceof CreatePublicKeyCredentialResponse) {
                                    answer(
                                            call,
                                            ((CreatePublicKeyCredentialResponse) response)
                                                    .getRegistrationResponseJson());
                                } else {
                                    call.reject("not a passkey", PasskeyFailure.FAILED);
                                }
                            }

                            @Override
                            public void onError(CreateCredentialException e) {
                                call.reject(e.getMessage(), PasskeyFailure.of(e));
                            }
                        });
    }

    @PluginMethod
    public void get(PluginCall call) {
        Activity activity = getActivity();
        JSObject publicKey = call.getObject("publicKey");
        if (activity == null || publicKey == null || Build.VERSION.SDK_INT < FIRST_SDK) {
            call.reject("Credential Manager cannot run here", PasskeyFailure.UNAVAILABLE);
            return;
        }
        CredentialManager.create(activity)
                .getCredentialAsync(
                        activity,
                        getRequest(publicKey.toString()),
                        null,
                        activity.getMainExecutor(),
                        new CredentialManagerCallback<
                                GetCredentialResponse, GetCredentialException>() {
                            @Override
                            public void onResult(GetCredentialResponse response) {
                                Credential credential = response.getCredential();
                                if (credential instanceof PublicKeyCredential) {
                                    answer(
                                            call,
                                            ((PublicKeyCredential) credential)
                                                    .getAuthenticationResponseJson());
                                } else {
                                    call.reject("not a passkey", PasskeyFailure.FAILED);
                                }
                            }

                            @Override
                            public void onError(GetCredentialException e) {
                                call.reject(e.getMessage(), PasskeyFailure.of(e));
                            }
                        });
    }

    /*
     * Both requests ask only for what is at hand on the phone. Otherwise, with no passkey for the
     * deployment, Google's sheet waits for the person to dismiss it, which reads as cancelled; at
     * hand only, it fails at once with `unavailable`, and the system browser, which also reaches
     * security keys and passkeys on another phone, takes the ceremony.
     */

    static CreatePublicKeyCredentialRequest createRequest(String publicKey) {
        return new CreatePublicKeyCredentialRequest(publicKey, null, true);
    }

    static GetCredentialRequest getRequest(String publicKey) {
        return new GetCredentialRequest.Builder()
                .addCredentialOption(new GetPublicKeyCredentialOption(publicKey))
                .setPreferImmediatelyAvailableCredentials(true)
                .build();
    }

    private static void answer(PluginCall call, String json) {
        try {
            JSObject answer = new JSObject();
            answer.put("credential", new JSObject(json));
            call.resolve(answer);
        } catch (JSONException e) {
            call.reject("the provider answered no JSON", PasskeyFailure.FAILED);
        }
    }

    /**
     * The certificates the installed app is signed with: its signers, or every certificate in
     * its rotation history, any of which a deployment may name.
     */
    private List<Signature> signers() {
        try {
            PackageInfo info =
                    getContext()
                            .getPackageManager()
                            .getPackageInfo(
                                    getContext().getPackageName(),
                                    PackageManager.GET_SIGNING_CERTIFICATES);
            SigningInfo signing = info.signingInfo;
            if (signing == null) {
                return Collections.emptyList();
            }
            Signature[] signatures =
                    signing.hasMultipleSigners()
                            ? signing.getApkContentsSigners()
                            : signing.getSigningCertificateHistory();
            return signatures == null ? Collections.emptyList() : List.of(signatures);
        } catch (PackageManager.NameNotFoundException e) {
            return Collections.emptyList();
        }
    }

    /** The SHA-256 of `certificate` as upper-case hex pairs joined by colons. */
    static String fingerprint(byte[] certificate) {
        try {
            byte[] digest = MessageDigest.getInstance("SHA-256").digest(certificate);
            StringBuilder hex = new StringBuilder(digest.length * 3);
            for (byte b : digest) {
                if (hex.length() > 0) {
                    hex.append(':');
                }
                hex.append(String.format(Locale.ROOT, "%02X", b & 0xff));
            }
            return hex.toString();
        } catch (NoSuchAlgorithmException e) {
            throw new IllegalStateException("every Android has SHA-256", e);
        }
    }
}
