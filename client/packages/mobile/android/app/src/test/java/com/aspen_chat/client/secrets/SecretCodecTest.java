package com.aspen_chat.client.secrets;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;

import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.util.Arrays;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import org.junit.Test;

public class SecretCodecTest {

    private static SecretKey key() throws GeneralSecurityException {
        KeyGenerator generator = KeyGenerator.getInstance("AES");
        generator.init(256);
        return generator.generateKey();
    }

    @Test
    public void opensWhatItSealedWithoutShowingIt() throws GeneralSecurityException {
        SecretKey key = key();
        byte[] sealed = SecretCodec.seal(key, "file/entry", "refresh-tökén");
        assertEquals("refresh-tökén", SecretCodec.open(key, "file/entry", sealed));
        assertFalse(
            new String(sealed, StandardCharsets.ISO_8859_1).contains("refresh")
        );
    }

    @Test
    public void sealsTheSameTextDifferentlyEachTime() throws GeneralSecurityException {
        SecretKey key = key();
        assertFalse(
            Arrays.equals(SecretCodec.seal(key, "c", "text"), SecretCodec.seal(key, "c", "text"))
        );
    }

    @Test
    public void refusesAnotherEntryAnotherKeyAndAlteredBytes() throws GeneralSecurityException {
        SecretKey key = key();
        byte[] sealed = SecretCodec.seal(key, "file/entry", "text");
        assertThrows(GeneralSecurityException.class, () -> SecretCodec.open(key, "file/other", sealed));
        assertThrows(GeneralSecurityException.class, () -> SecretCodec.open(key(), "file/entry", sealed));
        byte[] altered = sealed.clone();
        altered[altered.length - 1] ^= 1;
        assertThrows(GeneralSecurityException.class, () -> SecretCodec.open(key, "file/entry", altered));
        assertThrows(
            GeneralSecurityException.class,
            () -> SecretCodec.open(key, "file/entry", Arrays.copyOf(sealed, 12))
        );
    }
}
