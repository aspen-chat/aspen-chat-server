import type { Session } from "@aspen/protocol";
import { useRouter } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { Dialog, Modal, ModalOverlay } from "react-aria-components";
import { useAccountsHub, type AccountEntry } from "@/api/accounts";
import { AspenClientContext, HomeClientContext } from "@/api/context";
import { detectShell } from "@/config";
import { DeploymentWelcome } from "@/features/auth/DeploymentWelcome";
import { LoginForm } from "@/features/auth/LoginForm";
import { RegisterForm } from "@/features/auth/RegisterForm";
import { ServerChoiceContext } from "@/features/auth/serverChoice";
import { ServerForm } from "@/features/auth/ServerForm";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { toast } from "@/features/layout/toast";
import { useMessages } from "@/i18n/context";
import { AddingAccountContext } from "./addingAccount";

/**
 * Signs in to one more account with the app's own sign-in forms, against a client of its own. The
 * web client asks for no server: its page comes from its deployment, which must not be handed
 * another deployment's password or sessions.
 */
export function AddAccountDialog({
  isOpen,
  onOpenChange,
}: {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const accounts = useAccountsHub();
  const draft = useRef<AccountEntry | null>(null);
  const close = () => {
    if (draft.current !== null) {
      accounts.discard(draft.current);
      draft.current = null;
    }
    onOpenChange(false);
  };
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={(open) => {
        if (open) {
          onOpenChange(true);
        } else {
          close();
        }
      }}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass} aria-label={m.accounts.addHeading}>
          <DialogHeading>{m.accounts.addHeading}</DialogHeading>
          <Adding
            draft={draft}
            onDone={() => {
              draft.current = null;
              onOpenChange(false);
            }}
          />
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function Adding({
  draft,
  onDone,
}: {
  draft: { current: AccountEntry | null };
  onDone: () => void;
}) {
  const accounts = useAccountsHub();
  const [serverUrl, setServerUrl] = useState<string | null>(
    detectShell() === "web" ? accounts.chosenServer : null,
  );
  if (serverUrl === null) {
    return <ServerForm onSubmit={setServerUrl} offered={null} />;
  }
  draft.current ??= accounts.draft(serverUrl);
  return <SigningIn entry={draft.current} onDone={onDone} />;
}

function SigningIn({ entry, onDone }: { entry: AccountEntry; onDone: () => void }) {
  const m = useMessages();
  const accounts = useAccountsHub();
  const router = useRouter();
  const [registering, setRegistering] = useState(false);
  const { client, account } = entry;
  useEffect(() => {
    const signedIn = (session: Session | null) => {
      if (session === null) {
        return;
      }
      const held = accounts
        .view()
        .entries.find(
          (other) =>
            other.account.serverUrl === account.serverUrl &&
            other.client.session?.userId === session.userId,
        );
      if (held !== undefined) {
        accounts.discard(entry);
        accounts.bringToFront(held.account.id);
        router.history.push(accounts.routeOf(held.account.id) ?? "/");
        toast(m.accounts.already);
      } else if (accounts.commit(entry)) {
        router.history.push("/");
      } else {
        accounts.discard(entry);
        toast(m.accounts.full);
      }
      onDone();
    };
    return client.subscribe(signedIn);
  }, [accounts, account, client, entry, m, onDone, router]);
  return (
    <AddingAccountContext.Provider value={true}>
      <ServerChoiceContext.Provider
        value={{ serverUrl: account.serverUrl, changeServer: null, switchServer: null }}
      >
        <HomeClientContext.Provider value={client}>
          <AspenClientContext.Provider value={client}>
            <div className="flex flex-col items-center gap-4">
              <DeploymentWelcome serverUrl={account.serverUrl} onChangeServer={null} />
              {registering ? (
                <RegisterForm
                  onSwitchToLogin={() => {
                    setRegistering(false);
                  }}
                />
              ) : (
                <LoginForm
                  onSwitchToRegister={() => {
                    setRegistering(true);
                  }}
                />
              )}
            </div>
          </AspenClientContext.Provider>
        </HomeClientContext.Provider>
      </ServerChoiceContext.Provider>
    </AddingAccountContext.Provider>
  );
}
