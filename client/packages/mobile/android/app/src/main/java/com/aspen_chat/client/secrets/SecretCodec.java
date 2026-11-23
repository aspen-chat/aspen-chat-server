package com.aspen_chat.client.secrets;

import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import javax.crypto.Cipher;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/**
 * Seals text with AES-GCM under a key it never sees the bytes of: the cipher's random nonce, then
 * the ciphertext. {@code context} is authenticated, so a value moved to another entry does not
 * open there.
 */
final class SecretCodec {

    private static final String TRANSFORMATION = "AES/GCM/NoPadding";
    private static final int NONCE_BYTES = 12;
    private static final int TAG_BITS = 128;

    private SecretCodec() {}

    static byte[] seal(SecretKey key, String context, String text) throws GeneralSecurityException {
        Cipher cipher = Cipher.getInstance(TRANSFORMATION);
        cipher.init(Cipher.ENCRYPT_MODE, key);
        cipher.updateAAD(context.getBytes(StandardCharsets.UTF_8));
        byte[] sealed = cipher.doFinal(text.getBytes(StandardCharsets.UTF_8));
        byte[] nonce = cipher.getIV();
        if (nonce.length != NONCE_BYTES) {
            throw new GeneralSecurityException("unexpected nonce length " + nonce.length);
        }
        return ByteBuffer.allocate(nonce.length + sealed.length).put(nonce).put(sealed).array();
    }

    static String open(SecretKey key, String context, byte[] value) throws GeneralSecurityException {
        if (value.length <= NONCE_BYTES) {
            throw new GeneralSecurityException("sealed value too short");
        }
        Cipher cipher = Cipher.getInstance(TRANSFORMATION);
        cipher.init(Cipher.DECRYPT_MODE, key, new GCMParameterSpec(TAG_BITS, value, 0, NONCE_BYTES));
        cipher.updateAAD(context.getBytes(StandardCharsets.UTF_8));
        byte[] text = cipher.doFinal(value, NONCE_BYTES, value.length - NONCE_BYTES);
        return new String(text, StandardCharsets.UTF_8);
    }
}
