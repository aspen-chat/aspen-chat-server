package org.aspenchat.client.push;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;

import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.util.Base64;
import org.junit.Test;

public class WebPushTest {

    private static byte[] b64(String value) {
        return Base64.getUrlDecoder().decode(value);
    }

    private static final byte[] UA_PUBLIC = b64(
        "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4"
    );
    private static final byte[] UA_PRIVATE = b64("q1dXpw3UpT5VOmu_cf_v6ih07Aems3njxI-JWgLcM94");
    private static final byte[] AUTH = b64("BTBZMqHH6r4Tts7J_aSIgg");
    private static final byte[] HEADER = b64(
        "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8"
    );
    private static final byte[] CIPHERTEXT = b64("8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ");

    private static byte[] message() {
        byte[] message = new byte[HEADER.length + CIPHERTEXT.length];
        System.arraycopy(HEADER, 0, message, 0, HEADER.length);
        System.arraycopy(CIPHERTEXT, 0, message, HEADER.length, CIPHERTEXT.length);
        return message;
    }

    /** RFC 8291, Appendix A. */
    @Test
    public void readsTheRfcExample() throws GeneralSecurityException {
        byte[] plaintext = WebPush.decrypt(message(), UA_PRIVATE, UA_PUBLIC, AUTH);
        assertEquals("When I grow up, I want to be a watermelon", new String(plaintext, StandardCharsets.UTF_8));
    }

    @Test
    public void refusesATamperedMessage() {
        byte[] tampered = message();
        tampered[tampered.length - 1] ^= 1;
        assertThrows(GeneralSecurityException.class, () -> WebPush.decrypt(tampered, UA_PRIVATE, UA_PUBLIC, AUTH));
    }

    @Test
    public void refusesTheWrongSecret() {
        assertThrows(GeneralSecurityException.class, () -> WebPush.decrypt(message(), UA_PRIVATE, UA_PUBLIC, new byte[16]));
    }
}
