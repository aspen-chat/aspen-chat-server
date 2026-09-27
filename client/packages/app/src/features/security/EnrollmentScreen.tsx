import { SignOutIcon } from "@phosphor-icons/react";
import { Button } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { usePasskeyTransport } from "@/features/auth/passkeyTransport";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";
import { SecurityPanel } from "./SecurityPanel";

/**
 * Shown instead of the app while the server requires a second factor the account lacks: the
 * session can do nothing else until one is added, which lifts the requirement and shows the app.
 */
export function EnrollmentScreen() {
  const m = useMessages();
  const client = useAspenClient();
  const transport = usePasskeyTransport();
  return (
    <main className="flex min-h-full items-center justify-center p-6">
      <div className="flex w-full max-w-md flex-col gap-4">
        <h1 className="text-2xl font-semibold">{m.security.enrollmentHeading}</h1>
        <p className="text-sm text-ink-muted">{m.security.enrollmentPrompt}</p>
        <SecurityPanel transport={transport} />
        <Button
          onPress={() => {
            void client.logout();
          }}
          className={secondaryButtonClass + " flex items-center gap-1.5 self-start text-danger"}
        >
          <SignOutIcon size={16} aria-hidden="true" />
          {m.signOut}
        </Button>
      </div>
    </main>
  );
}
