package org.aspenchat.client.push;

import java.math.BigInteger;
import java.security.AlgorithmParameters;
import java.security.GeneralSecurityException;
import java.security.KeyFactory;
import java.security.PrivateKey;
import java.security.PublicKey;
import java.security.spec.ECGenParameterSpec;
import java.security.spec.ECParameterSpec;
import java.security.spec.ECPoint;
import java.security.spec.ECPrivateKeySpec;
import java.security.spec.ECPublicKeySpec;
import java.util.Arrays;
import javax.crypto.Cipher;
import javax.crypto.KeyAgreement;
import javax.crypto.Mac;
import javax.crypto.spec.GCMParameterSpec;
import javax.crypto.spec.SecretKeySpec;

/**
 * Decrypts a Web Push message (RFC 8291: one {@code aes128gcm} record, RFC 8188) with one
 * subscription's keys, as {@code decryptWebPush} in the protocol package does.
 */
public final class WebPush {

    private WebPush() {}

    /**
     * @param message the whole message: salt, record size, the sender's key, and the record
     * @param privateKey the subscription's private scalar ({@code d} of its JWK)
     * @param publicKey the subscription's public key, an uncompressed P-256 point
     * @param auth the subscription's 16-byte authentication secret
     */
    public static byte[] decrypt(byte[] message, byte[] privateKey, byte[] publicKey, byte[] auth)
        throws GeneralSecurityException {
        if (message.length < 21) {
            throw new GeneralSecurityException("a push too short to be one");
        }
        byte[] salt = Arrays.copyOfRange(message, 0, 16);
        int idLength = message[20] & 0xff;
        if (message.length < 21 + idLength) {
            throw new GeneralSecurityException("a push too short for its key");
        }
        byte[] senderKey = Arrays.copyOfRange(message, 21, 21 + idLength);
        byte[] record = Arrays.copyOfRange(message, 21 + idLength, message.length);

        ECParameterSpec curve = p256();
        KeyFactory factory = KeyFactory.getInstance("EC");
        PrivateKey ours = factory.generatePrivate(new ECPrivateKeySpec(new BigInteger(1, privateKey), curve));
        PublicKey theirs = factory.generatePublic(new ECPublicKeySpec(point(senderKey), curve));
        KeyAgreement agreement = KeyAgreement.getInstance("ECDH");
        agreement.init(ours);
        agreement.doPhase(theirs, true);
        byte[] secret = agreement.generateSecret();

        byte[] ikm = hkdf(auth, secret, concat(ascii("WebPush: info\0"), publicKey, senderKey), 32);
        byte[] cek = hkdf(salt, ikm, ascii("Content-Encoding: aes128gcm\0"), 16);
        byte[] nonce = hkdf(salt, ikm, ascii("Content-Encoding: nonce\0"), 12);
        Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
        cipher.init(Cipher.DECRYPT_MODE, new SecretKeySpec(cek, "AES"), new GCMParameterSpec(128, nonce));
        byte[] padded = cipher.doFinal(record);
        // The record ends with its delimiter, 2 for the last record, then any zero padding.
        int end = padded.length;
        while (end > 0 && padded[end - 1] == 0) {
            end--;
        }
        if (end == 0 || padded[end - 1] != 2) {
            throw new GeneralSecurityException("a push's record does not end as the last one does");
        }
        return Arrays.copyOf(padded, end - 1);
    }

    static ECParameterSpec p256() throws GeneralSecurityException {
        AlgorithmParameters parameters = AlgorithmParameters.getInstance("EC");
        parameters.init(new ECGenParameterSpec("secp256r1"));
        return parameters.getParameterSpec(ECParameterSpec.class);
    }

    static ECPoint point(byte[] uncompressed) throws GeneralSecurityException {
        if (uncompressed.length != 65 || uncompressed[0] != 4) {
            throw new GeneralSecurityException("not an uncompressed P-256 point");
        }
        return new ECPoint(
            new BigInteger(1, Arrays.copyOfRange(uncompressed, 1, 33)),
            new BigInteger(1, Arrays.copyOfRange(uncompressed, 33, 65))
        );
    }

    /** HKDF-SHA256 (RFC 5869), expanded to at most one block. */
    static byte[] hkdf(byte[] salt, byte[] ikm, byte[] info, int length) throws GeneralSecurityException {
        Mac mac = Mac.getInstance("HmacSHA256");
        mac.init(new SecretKeySpec(salt, "HmacSHA256"));
        byte[] prk = mac.doFinal(ikm);
        mac.init(new SecretKeySpec(prk, "HmacSHA256"));
        mac.update(info);
        mac.update((byte) 1);
        return Arrays.copyOf(mac.doFinal(), length);
    }

    private static byte[] ascii(String text) {
        return text.getBytes(java.nio.charset.StandardCharsets.US_ASCII);
    }

    private static byte[] concat(byte[]... parts) {
        int length = 0;
        for (byte[] part : parts) {
            length += part.length;
        }
        byte[] out = new byte[length];
        int at = 0;
        for (byte[] part : parts) {
            System.arraycopy(part, 0, out, at, part.length);
            at += part.length;
        }
        return out;
    }
}
