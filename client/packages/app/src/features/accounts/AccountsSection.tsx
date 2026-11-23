import { MAX_ACCOUNTS } from "@aspen/protocol";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useAccounts, useAccountsHub, type AccountEntry } from "@/api/accounts";
import { useSignOut } from "@/api/deploymentsContext";
import { detectShell } from "@/config";
import { dangerButtonClass, planeClass, secondaryButtonClass } from "@/features/invites/dialog";
import { ChoiceCheckbox } from "@/features/layout/choices";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { AccountPicture } from "./AccountPicture";
import { AddAccountDialog } from "./AddAccountDialog";
import { accountHost, useAccountSummary } from "./useAccountSummary";
import { useSwitchAccount } from "./useSwitchAccount";

/**
 * Whether this install holds several accounts, and which. It is the install's choice rather than
 * an account's, so it is kept with the list of accounts, before any of them has signed in.
 */
export function AccountsSection() {
  const m = useMessages().accounts;
  const hub = useAccountsHub();
  const { enabled, front, entries } = useAccounts();
  const [confirmingOff, setConfirmingOff] = useState(false);
  const [adding, setAdding] = useState(false);
  const others = entries.length - 1;
  return (
    <section aria-labelledby="settings-accounts" className={planeClass}>
      <h3 id="settings-accounts" className="text-lg font-semibold text-ink-muted">
        {m.heading}
      </h3>
      <ChoiceCheckbox
        isSelected={enabled}
        onChange={(selected) => {
          if (selected) {
            hub.enable();
          } else if (others > 0) {
            setConfirmingOff(true);
          } else {
            void hub.disable();
          }
        }}
        label={m.useSeveral}
        hint={detectShell() === "web" ? m.useSeveralHintWeb : m.useSeveralHint}
      />
      {confirmingOff && (
        <div role="alert" className="flex flex-col gap-2 text-sm">
          <p>{format(m.turnOffConfirm, { count: others })}</p>
          <div className="flex flex-wrap gap-2">
            <Button
              className={dangerButtonClass}
              onPress={() => {
                setConfirmingOff(false);
                void hub.disable();
              }}
            >
              {m.turnOff}
            </Button>
            <Button
              className={secondaryButtonClass}
              onPress={() => {
                setConfirmingOff(false);
              }}
            >
              {m.keep}
            </Button>
          </div>
        </div>
      )}
      {enabled && (
        <>
          <ul className="flex flex-col gap-1">
            {entries.map((entry) => (
              <AccountRow
                key={entry.account.id}
                entry={entry}
                front={entry.account.id === front}
                alone={entries.length === 1}
              />
            ))}
          </ul>
          {entries.length < MAX_ACCOUNTS ? (
            <Button
              className={secondaryButtonClass}
              onPress={() => {
                setAdding(true);
              }}
            >
              {m.add}
            </Button>
          ) : (
            <p className="text-sm text-ink-muted">{format(m.limit, { count: MAX_ACCOUNTS })}</p>
          )}
          <AddAccountDialog isOpen={adding} onOpenChange={setAdding} />
        </>
      )}
    </section>
  );
}

function AccountRow({
  entry,
  front,
  alone,
}: {
  entry: AccountEntry;
  front: boolean;
  alone: boolean;
}) {
  const m = useMessages().accounts;
  const summary = useAccountSummary(entry);
  const switchTo = useSwitchAccount();
  const signOut = useSignOut();
  const [confirming, setConfirming] = useState(false);
  const name = summary.name ?? accountHost(entry);
  return (
    <li className="flex flex-col gap-2 rounded-md border border-line px-3 py-2">
      <span className="flex items-center gap-2">
        <AccountPicture entry={entry} summary={summary} size="sm" />
        <span className="flex min-w-0 flex-1 flex-col text-sm">
          <span className="truncate font-medium">{name}</span>
          <span className="truncate text-xs text-ink-muted">
            {summary.signedIn ? (summary.handle ?? accountHost(entry)) : m.signedOut}
          </span>
        </span>
      </span>
      <span className="flex flex-wrap items-center gap-2">
        {front ? (
          <span className="text-xs text-ink-muted">{m.current}</span>
        ) : (
          <Button
            aria-label={format(m.switchToNamed, { name })}
            className={secondaryButtonClass}
            onPress={() => {
              switchTo(entry.account.id);
            }}
          >
            {m.switchTo}
          </Button>
        )}
        {!alone && (
          <Button
            aria-label={format(m.signOutNamed, { name })}
            className={confirming ? dangerButtonClass : secondaryButtonClass + " text-danger"}
            onPress={() => {
              if (confirming) {
                void signOut(entry.account.id);
              } else {
                setConfirming(true);
              }
            }}
          >
            {confirming ? m.signOutConfirm : format(m.signOutNamed, { name })}
          </Button>
        )}
      </span>
    </li>
  );
}
