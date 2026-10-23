import {
  passkeySignalAvailable,
  signalAllAcceptedCredentials,
  signalCurrentUserDetails,
  type AspenClient,
  type PasskeyTransport,
  type User,
} from "@aspen/protocol";
import { useEffect } from "react";
import { fetchSecurity, type SecuritySettings } from "@/features/security/api";

/**
 * Tells the browser's passkey managers every passkey the account holds, whenever its security
 * settings are read: as they open, and after a passkey is added, renamed, or removed. Managers
 * may then hide the account's others, which no longer sign anyone in. Only a page under the
 * relying party's domain may (`inPage`); `settings` lists every passkey, as the signal needs.
 */
export function useSignalAcceptedPasskeys(
  settings: SecuritySettings,
  transport: PasskeyTransport | null,
): void {
  useEffect(() => {
    if (transport?.kind === "inPage") {
      signalAllAcceptedCredentials(
        transport.rpId,
        settings.passkeyUserHandle,
        settings.passkeys.map((passkey) => passkey.credentialId),
      );
    }
  }, [settings, transport]);
}

/**
 * Tells the browser's passkey managers the account's username and name as `user` now has them,
 * after either changed, so they show them beside its passkeys. The account's user handle is in
 * its security settings, read only where the browser has the signal and the page may send it,
 * and nothing is sent for an account without passkeys.
 */
export async function signalProfileToPasskeys(
  client: AspenClient,
  transport: PasskeyTransport | null,
  user: User,
): Promise<void> {
  if (transport?.kind !== "inPage" || !passkeySignalAvailable("signalCurrentUserDetails")) {
    return;
  }
  const settings = await fetchSecurity(client).catch(() => null);
  if (settings === null || settings.passkeys.length === 0) {
    return;
  }
  signalCurrentUserDetails(
    transport.rpId,
    settings.passkeyUserHandle,
    user.name,
    user.displayName ?? user.name,
  );
}
