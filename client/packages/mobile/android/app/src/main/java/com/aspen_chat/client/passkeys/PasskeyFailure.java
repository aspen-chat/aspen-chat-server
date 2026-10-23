package com.aspen_chat.client.passkeys;

import androidx.credentials.exceptions.CreateCredentialCancellationException;
import androidx.credentials.exceptions.CreateCredentialNoCreateOptionException;
import androidx.credentials.exceptions.CreateCredentialProviderConfigurationException;
import androidx.credentials.exceptions.CreateCredentialUnsupportedException;
import androidx.credentials.exceptions.GetCredentialCancellationException;
import androidx.credentials.exceptions.GetCredentialProviderConfigurationException;
import androidx.credentials.exceptions.GetCredentialUnsupportedException;
import androidx.credentials.exceptions.NoCredentialException;
import androidx.credentials.exceptions.domerrors.DomError;
import androidx.credentials.exceptions.domerrors.NotAllowedError;
import androidx.credentials.exceptions.domerrors.NotSupportedError;
import androidx.credentials.exceptions.domerrors.SecurityError;
import androidx.credentials.exceptions.publickeycredential.CreatePublicKeyCredentialDomException;
import androidx.credentials.exceptions.publickeycredential.GetPublicKeyCredentialDomException;

/**
 * What a failed Credential Manager call means to the page, as the code it rejects with
 * (`nativePasskeys.ts` in the app reads them).
 */
final class PasskeyFailure {

    /** The person dismissed the prompt, or let it time out: the ceremony ends there. */
    static final String CANCELLED = "cancelled";

    /**
     * Nothing here can run it: no provider is set up or answered, the platform lacks Credential
     * Manager, or it refused this app for the deployment (the deployment's
     * `assetlinks.json` does not name it, or Google has not read it yet). The page hands the
     * ceremony to the system browser instead.
     */
    static final String UNAVAILABLE = "unavailable";

    /** Anything else, which the page reports as a failed ceremony. */
    static final String FAILED = "failed";

    private PasskeyFailure() {}

    static String of(Exception e) {
        if (e instanceof GetCredentialCancellationException
                || e instanceof CreateCredentialCancellationException) {
            return CANCELLED;
        }
        if (e instanceof NoCredentialException
                || e instanceof CreateCredentialNoCreateOptionException
                || e instanceof GetCredentialProviderConfigurationException
                || e instanceof CreateCredentialProviderConfigurationException
                || e instanceof GetCredentialUnsupportedException
                || e instanceof CreateCredentialUnsupportedException) {
            return UNAVAILABLE;
        }
        DomError dom = null;
        if (e instanceof GetPublicKeyCredentialDomException) {
            dom = ((GetPublicKeyCredentialDomException) e).getDomError();
        } else if (e instanceof CreatePublicKeyCredentialDomException) {
            dom = ((CreatePublicKeyCredentialDomException) e).getDomError();
        }
        if (dom instanceof NotAllowedError) {
            // As a browser reports a dismissed prompt.
            return CANCELLED;
        }
        if (dom instanceof SecurityError || dom instanceof NotSupportedError) {
            return UNAVAILABLE;
        }
        return FAILED;
    }
}
