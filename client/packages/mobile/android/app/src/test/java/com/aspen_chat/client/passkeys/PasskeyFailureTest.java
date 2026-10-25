package com.aspen_chat.client.passkeys;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import androidx.credentials.exceptions.CreateCredentialCancellationException;
import androidx.credentials.exceptions.CreateCredentialProviderConfigurationException;
import androidx.credentials.exceptions.CreateCredentialUnknownException;
import androidx.credentials.exceptions.GetCredentialCancellationException;
import androidx.credentials.exceptions.GetCredentialUnsupportedException;
import androidx.credentials.exceptions.NoCredentialException;
import androidx.credentials.exceptions.domerrors.InvalidStateError;
import androidx.credentials.exceptions.domerrors.NotAllowedError;
import androidx.credentials.exceptions.domerrors.SecurityError;
import androidx.credentials.exceptions.publickeycredential.CreatePublicKeyCredentialDomException;
import androidx.credentials.exceptions.publickeycredential.GetPublicKeyCredentialDomException;
import java.nio.charset.StandardCharsets;
import org.junit.Test;

public class PasskeyFailureTest {

    @Test
    public void aDismissedPromptEndsTheCeremony() {
        assertEquals(PasskeyFailure.CANCELLED, PasskeyFailure.of(new GetCredentialCancellationException()));
        assertEquals(PasskeyFailure.CANCELLED, PasskeyFailure.of(new CreateCredentialCancellationException()));
        assertEquals(
                PasskeyFailure.CANCELLED,
                PasskeyFailure.of(new GetPublicKeyCredentialDomException(new NotAllowedError())));
    }

    @Test
    public void whatNothingHereCanRunGoesToTheBrowser() {
        assertEquals(PasskeyFailure.UNAVAILABLE, PasskeyFailure.of(new NoCredentialException()));
        assertEquals(
                PasskeyFailure.UNAVAILABLE,
                PasskeyFailure.of(new CreateCredentialProviderConfigurationException()));
        assertEquals(PasskeyFailure.UNAVAILABLE, PasskeyFailure.of(new GetCredentialUnsupportedException()));
        // The platform refused the app for the domain: its asset links do not name it.
        assertEquals(
                PasskeyFailure.UNAVAILABLE,
                PasskeyFailure.of(new CreatePublicKeyCredentialDomException(new SecurityError())));
    }

    @Test
    public void anythingElseFails() {
        assertEquals(PasskeyFailure.FAILED, PasskeyFailure.of(new CreateCredentialUnknownException()));
        assertEquals(
                PasskeyFailure.FAILED,
                PasskeyFailure.of(new CreatePublicKeyCredentialDomException(new InvalidStateError())));
        assertEquals(PasskeyFailure.FAILED, PasskeyFailure.of(new IllegalStateException()));
    }

    @Test
    public void onlyTheDeploymentsOwnRelyingPartyRunsHere() {
        assertTrue(AspenPasskeysPlugin.rpIdFits("example.org", "example.org"));
        assertTrue(AspenPasskeysPlugin.rpIdFits("example.org", "chat.example.org"));
        assertTrue(AspenPasskeysPlugin.rpIdFits("Example.ORG", "chat.example.org"));
        assertFalse(AspenPasskeysPlugin.rpIdFits("victim.example", "evil.example"));
        assertFalse(AspenPasskeysPlugin.rpIdFits("example.org", "badexample.org"));
        assertFalse(AspenPasskeysPlugin.rpIdFits("chat.example.org", "example.org"));
        assertFalse(AspenPasskeysPlugin.rpIdFits("", "example.org"));
        assertFalse(AspenPasskeysPlugin.rpIdFits(null, "example.org"));
        assertFalse(AspenPasskeysPlugin.rpIdFits("example.org", null));
    }

    @Test
    public void fingerprintsAreWrittenAsDigitalAssetLinksWriteThem() {
        // SHA-256 of "abc".
        assertEquals(
                "BA:78:16:BF:8F:01:CF:EA:41:41:40:DE:5D:AE:22:23:B0:03:61:A3:96:17:7A:9C:B4:10:FF:61:F2:00:15:AD",
                AspenPasskeysPlugin.fingerprint("abc".getBytes(StandardCharsets.US_ASCII)));
    }
}
