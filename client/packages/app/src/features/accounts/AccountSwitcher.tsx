import { MAX_ACCOUNTS } from "@aspen/protocol";
import { CheckIcon, PlusIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button, Menu, MenuItem, MenuTrigger, Popover } from "react-aria-components";
import { useAccounts, type AccountEntry } from "@/api/accounts";
import { menuClass, menuItemClass, menuPopoverClass } from "@/features/layout/menu";
import { Tooltip } from "@/features/layout/Tooltip";
import { MentionBadge } from "@/features/mentions/MentionBadge";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { AccountPicture } from "./AccountPicture";
import { AddAccountDialog } from "./AddAccountDialog";
import { accountHost, useAccountSummary } from "./useAccountSummary";
import { useSwitchAccount } from "./useSwitchAccount";

const ADD = "add";

/**
 * The account in front, whose menu lists the others with what awaits them. Shown once several
 * accounts are turned on; the mark on it says another account has something unread.
 */
export function AccountSwitcher() {
  const m = useMessages().accounts;
  const { enabled, front, entries } = useAccounts();
  const switchTo = useSwitchAccount();
  const [adding, setAdding] = useState(false);
  const shown = entries.find(({ account }) => account.id === front);
  if (!enabled || shown === undefined) {
    return null;
  }
  return (
    <>
      <MenuTrigger>
        <Tooltip text={m.switch}>
          <Button
            aria-label={m.switch}
            className="relative mt-auto shrink-0 rounded-full outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
          >
            <FrontPicture entry={shown} />
            <OthersMark entries={entries.filter((entry) => entry !== shown)} />
          </Button>
        </Tooltip>
        <Popover placement="end bottom" className={menuPopoverClass}>
          <Menu
            aria-label={m.switch}
            className={menuClass}
            onAction={(key) => {
              if (key === ADD) {
                setAdding(true);
              } else {
                switchTo(String(key));
              }
            }}
          >
            {entries.map((entry) => (
              <AccountItem key={entry.account.id} entry={entry} front={entry === shown} />
            ))}
            {entries.length < MAX_ACCOUNTS && (
              <MenuItem id={ADD} textValue={m.add} className={menuItemClass}>
                <PlusIcon size={16} aria-hidden="true" />
                {m.add}
              </MenuItem>
            )}
          </Menu>
        </Popover>
      </MenuTrigger>
      <AddAccountDialog isOpen={adding} onOpenChange={setAdding} />
    </>
  );
}

function FrontPicture({ entry }: { entry: AccountEntry }) {
  const summary = useAccountSummary(entry);
  return <AccountPicture entry={entry} summary={summary} size="lg" />;
}

function OthersMark({ entries }: { entries: readonly AccountEntry[] }) {
  return entries.map((entry) => <OtherMark key={entry.account.id} entry={entry} />);
}

function OtherMark({ entry }: { entry: AccountEntry }) {
  const { tags, unread } = useAccountSummary(entry);
  if (tags > 0) {
    return (
      <MentionBadge
        count={tags}
        className="pointer-events-none absolute -end-1 -bottom-1 ring-2 ring-surface-rail"
      />
    );
  }
  return unread ? (
    <span
      aria-hidden="true"
      className="forced-fill absolute -end-0.5 -top-0.5 h-2.5 w-2.5 rounded-full bg-ink ring-2 ring-surface-rail"
    />
  ) : null;
}

function AccountItem({ entry, front }: { entry: AccountEntry; front: boolean }) {
  const m = useMessages().accounts;
  const summary = useAccountSummary(entry);
  const name = summary.name ?? accountHost(entry);
  const label =
    summary.tags > 0
      ? format(m.unreadTags, { name, count: summary.tags })
      : summary.unread
        ? format(m.unread, { name })
        : name;
  return (
    <MenuItem id={entry.account.id} textValue={label} aria-label={label} className={menuItemClass}>
      <AccountPicture entry={entry} summary={summary} size="sm" />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate font-medium">{name}</span>
        <span className="truncate text-xs text-ink-muted">
          {summary.signedIn ? (summary.handle ?? accountHost(entry)) : m.signedOut}
        </span>
      </span>
      {summary.tags > 0 && <MentionBadge count={summary.tags} />}
      {front && <CheckIcon size={16} aria-label={m.current} />}
    </MenuItem>
  );
}
