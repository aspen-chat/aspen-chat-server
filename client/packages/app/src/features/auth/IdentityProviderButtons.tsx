import type { IdentityProviderOutcome } from "@aspen/protocol";
import { SignInIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button } from "react-aria-components";
import { detectShell } from "@/config";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { useIdentityProviders, useProviderFlow } from "./identityProviders";
import { outlineButtonClass } from "./styles";

/**
 * "Continue with {name}" for each identity provider the server offers, beside the passkey
 * button. The web client goes to the provider in this tab; the apps wait for the system
 * browser and hand on what it did (`onOutcome`), or why it failed (`onError`, which a person
 * backing out does not reach).
 */
export function IdentityProviderButtons({
  isDisabled,
  onStart,
  onOutcome,
  onError,
}: {
  isDisabled: boolean;
  onStart: () => void;
  onOutcome: (outcome: IdentityProviderOutcome) => void;
  onError: (e: unknown) => void;
}) {
  const m = useMessages();
  const providers = useIdentityProviders();
  const run = useProviderFlow();
  const [pending, setPending] = useState<string | null>(null);
  const web = detectShell() === "web";
  return providers.map((provider) => (
    <Button
      key={provider.id}
      isDisabled={isDisabled || pending !== null}
      onPress={() => {
        onStart();
        setPending(provider.id);
        run(provider, "signIn")
          .then(onOutcome, onError)
          .finally(() => {
            setPending(null);
          });
      }}
      className={outlineButtonClass}
    >
      <SignInIcon size={18} aria-hidden="true" className="rtl:-scale-x-100" />
      {pending === provider.id
        ? web
          ? format(m.identityProviders.goingTo, { provider: provider.name })
          : m.passkeyWaitingBrowser
        : format(m.identityProviders.continueWith, { provider: provider.name })}
    </Button>
  ));
}
