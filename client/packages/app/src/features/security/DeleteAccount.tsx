import { useState } from "react";
import { Button } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { useSignOut } from "@/api/deploymentsContext";
import { useMe } from "@/api/hooks";
import { usePasskeyTransport } from "@/features/auth/passkeyTransport";
import { ConfirmModal } from "@/features/layout/ConfirmModal";
import { useMessages } from "@/i18n/context";
import * as api from "./api";
import { ReauthProvider } from "./reauth";
import { useReauth } from "./reauthContext";
import { useSecuritySettings } from "./useSecuritySettings";

/**
 * Deleting the account, after a confirmation saying what it does. It takes a recent
 * verification, so it must be inside a `ReauthProvider`, and once it is done the app signs out
 * everywhere it was signed in from here, as the server has ended every sign-in.
 */
export function DeleteAccountButton({
  className,
  isDisabled = false,
  ownsNone = false,
}: {
  className: string;
  isDisabled?: boolean;
  /** The account is in no community, so owns none whose ownership the confirmation would tell of. */
  ownsNone?: boolean;
}) {
  const m = useMessages();
  const client = useAspenClient();
  const withReauth = useReauth();
  const signOut = useSignOut();
  const [confirming, setConfirming] = useState(false);
  return (
    <>
      <Button
        isDisabled={isDisabled}
        onPress={() => {
          setConfirming(true);
        }}
        className={className}
      >
        {m.security.deleteAccount}
      </Button>
      <ConfirmModal
        isOpen={confirming}
        onOpenChange={setConfirming}
        heading={m.security.deleteAccountConfirmHeading}
        explanation={
          ownsNone
            ? m.security.deleteAccountExplanationOwningNone
            : m.security.deleteAccountExplanation
        }
        action={m.security.deleteAccountAction}
        run={async () => {
          // `withReauth` answers `undefined` when the user declines to verify.
          const deleted = await withReauth(async () => {
            await api.deleteAccount(client);
            return true;
          });
          if (deleted === true) {
            await signOut();
          }
        }}
      />
    </>
  );
}

/**
 * `DeleteAccountButton` away from the security settings, loading what verifying needs. It is
 * disabled until that has loaded, since a verification asked for before then could not be
 * shown, and absent for a user of another deployment, who deletes their account at home.
 */
export function StandaloneDeleteAccountButton({
  className,
  ownsNone = false,
}: {
  className: string;
  ownsNone?: boolean;
}) {
  const me = useMe();
  const transport = usePasskeyTransport();
  const { settings } = useSecuritySettings();
  if (me?.homeDomain != null) {
    return null;
  }
  return (
    <ReauthProvider settings={settings} transport={transport}>
      <DeleteAccountButton
        className={className}
        isDisabled={settings === null}
        ownsNone={ownsNone}
      />
    </ReauthProvider>
  );
}
