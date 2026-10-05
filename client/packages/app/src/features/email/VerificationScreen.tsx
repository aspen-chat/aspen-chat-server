import { SignOutIcon } from "@phosphor-icons/react";
import { Button } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";
import { EmailPanel } from "./EmailPanel";

/**
 * Shown instead of the app while the deployment requires a verified email address and the
 * account's is not verified: the session can do nothing else until it types the code mailed to
 * its address (or changes the address, and types the code mailed to the new one), which lifts
 * the requirement and shows the app.
 */
export function VerificationScreen() {
  const m = useMessages();
  const client = useAspenClient();
  return (
    <main className="flex min-h-full items-center justify-center p-6">
      <div className="flex w-full max-w-md flex-col gap-4">
        <h1 className="text-2xl font-semibold">{m.email.gateHeading}</h1>
        <p className="text-sm text-ink-muted">{m.email.gatePrompt}</p>
        <EmailPanel gate />
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
