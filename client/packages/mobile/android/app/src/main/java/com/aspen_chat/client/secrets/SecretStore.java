package com.aspen_chat.client.secrets;

import android.content.Context;
import android.content.SharedPreferences;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;
import android.util.Log;
import java.security.GeneralSecurityException;
import java.security.KeyStore;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;

/**
 * Text holding sign-ins, kept in private preferences sealed under a key in the Android Keystore,
 * which never leaves it and needs no unlocking by the person, so notification and upload code can
 * read it while the app is closed. A value written before sealing is sealed the first time it is
 * read.
 */
public final class SecretStore {

    /** What is known of an entry: {@code failed} when it is kept but could not be opened. */
    public static final class Read {

        public final String text;
        public final boolean failed;

        Read(String text, boolean failed) {
            this.text = text;
            this.failed = failed;
        }
    }

    private static final String TAG = "AspenSecrets";
    private static final String KEYSTORE = "AndroidKeyStore";
    private static final String KEY_ALIAS = "aspen_secrets";
    private static final String SEALED_PREFIX = "sealed1:";

    private final SharedPreferences preferences;
    private final String file;

    public SecretStore(Context context, String file) {
        this.file = file;
        preferences = context.getApplicationContext().getSharedPreferences(file, Context.MODE_PRIVATE);
    }

    private static synchronized SecretKey key() throws GeneralSecurityException {
        try {
            KeyStore store = KeyStore.getInstance(KEYSTORE);
            store.load(null);
            if (store.getKey(KEY_ALIAS, null) instanceof SecretKey existing) {
                return existing;
            }
        } catch (java.io.IOException e) {
            throw new GeneralSecurityException("the keystore could not be read", e);
        }
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, KEYSTORE);
        generator.init(
            new KeyGenParameterSpec.Builder(
                KEY_ALIAS,
                KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        );
        return generator.generateKey();
    }

    private String context(String name) {
        return file + "/" + name;
    }

    public synchronized Read read(String name) {
        String kept = preferences.getString(name, null);
        if (kept == null) {
            return new Read(null, false);
        }
        if (!kept.startsWith(SEALED_PREFIX)) {
            if (!write(name, kept)) {
                Log.w(TAG, "an unsealed entry of " + file + " could not be sealed yet");
            }
            return new Read(kept, false);
        }
        try {
            byte[] sealed = Base64.decode(kept.substring(SEALED_PREFIX.length()), Base64.NO_WRAP);
            return new Read(SecretCodec.open(key(), context(name), sealed), false);
        } catch (GeneralSecurityException | IllegalArgumentException e) {
            Log.e(TAG, "an entry of " + file + " could not be opened", e);
            return new Read(null, true);
        }
    }

    /** The entry's text, {@code null} when there is none or it cannot be opened. */
    public String load(String name) {
        return read(name).text;
    }

    /** Forgets the entry {@code name}, as before it was written. */
    public synchronized void remove(String name) {
        preferences.edit().remove(name).apply();
    }

    /** Whether {@code text} is on disk, sealed; nothing is written unsealed. */
    public synchronized boolean write(String name, String text) {
        try {
            byte[] sealed = SecretCodec.seal(key(), context(name), text);
            String kept = SEALED_PREFIX + Base64.encodeToString(sealed, Base64.NO_WRAP);
            return preferences.edit().putString(name, kept).commit();
        } catch (GeneralSecurityException e) {
            Log.e(TAG, "an entry of " + file + " could not be sealed", e);
            return false;
        }
    }
}
