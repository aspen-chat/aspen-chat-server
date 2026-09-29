import type { User } from "@aspen/protocol";
import { ApiProblemError } from "@aspen/protocol";
import { useMemo, useState, type ReactNode } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Input,
  ListBox,
  ListBoxItem,
  Modal,
  ModalOverlay,
  SearchField,
  type Selection,
} from "react-aria-components";
import { usePeople, useUsers } from "@/api/hooks";
import { primaryButtonClass } from "@/features/auth/styles";
import { Avatar } from "@/features/communities/Avatar";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import { displayNameOf, handleOf } from "@/features/users/profile";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { SourceScope } from "@/api/deployments";
import type { Source } from "@/api/everywhere";

/**
 * A dialog for choosing people the caller shares a community with: whom to start a DM with,
 * or whom to add to a group. `exclude` are left out (the group's members, say) and at most
 * `max` may be chosen. `onConfirm` runs with the choice; the dialog closes once it resolves
 * and shows its error otherwise.
 */
export function PeoplePicker({
  trigger,
  heading,
  confirmLabel,
  pendingLabel,
  exclude,
  max,
  onConfirm,
  above,
  source,
}: {
  trigger: ReactNode;
  heading: string;
  confirmLabel: string;
  pendingLabel: string;
  exclude: readonly string[];
  max: number;
  onConfirm: (ids: readonly string[]) => Promise<void>;
  /** Shown under the heading, above the people. */
  above?: ReactNode;
  /** The deployment whose people are offered; the one shown when absent. */
  source?: Source;
}) {
  return (
    <DialogTrigger>
      {trigger}
      <ModalOverlay isDismissable className={overlayClass}>
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            {({ close }) => {
              const body = (
                <PickerBody
                  // Another deployment's people are other people: the picks start over.
                  key={source?.domain ?? ""}
                  heading={heading}
                  above={above}
                  confirmLabel={confirmLabel}
                  pendingLabel={pendingLabel}
                  exclude={exclude}
                  max={max}
                  onConfirm={onConfirm}
                  close={close}
                />
              );
              return source === undefined ? (
                body
              ) : (
                <SourceScope source={source}>{body}</SourceScope>
              );
            }}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

function PickerBody({
  heading,
  above,
  confirmLabel,
  pendingLabel,
  exclude,
  max,
  onConfirm,
  close,
}: {
  heading: string;
  above: ReactNode;
  confirmLabel: string;
  pendingLabel: string;
  exclude: readonly string[];
  max: number;
  onConfirm: (ids: readonly string[]) => Promise<void>;
  close: () => void;
}) {
  const m = useMessages();
  const people = usePeople();
  const candidates = useMemo(() => people.filter((id) => !exclude.includes(id)), [people, exclude]);
  const users = useUsers(candidates);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const listed = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return users
      .filter((user): user is User => user !== undefined)
      .filter(
        (user) =>
          needle === "" ||
          displayNameOf(user).toLowerCase().includes(needle) ||
          user.name.toLowerCase().includes(needle),
      )
      .sort((a, b) => displayNameOf(a).localeCompare(displayNameOf(b)));
  }, [users, query]);

  const tooMany = selected.size > max;
  const confirm = async () => {
    setPending(true);
    setError(null);
    try {
      await onConfirm(Array.from(selected));
      close();
    } catch (failure) {
      setError(failure instanceof ApiProblemError ? failure.message : String(failure));
      setPending(false);
    }
  };

  return (
    <>
      <DialogHeading>{heading}</DialogHeading>
      {above}
      <SearchField
        aria-label={m.dms.search}
        value={query}
        onChange={setQuery}
        className="flex flex-col"
      >
        <Input
          placeholder={m.dms.search}
          className="rounded-md border border-line bg-surface px-3 py-2 text-sm outline-none focus:border-accent focus:ring-2 focus:ring-accent/30"
        />
      </SearchField>
      {candidates.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.dms.noPeople}</p>
      ) : (
        <ListBox
          aria-label={m.dms.people}
          items={listed}
          selectionMode="multiple"
          selectedKeys={selected}
          onSelectionChange={(keys: Selection) => {
            setSelected(
              keys === "all" ? new Set(listed.map((u) => u.id)) : new Set(Array.from(keys, String)),
            );
          }}
          className="max-h-72 overflow-y-auto rounded-md border border-line"
        >
          {(user) => (
            <ListBoxItem
              id={user.id}
              textValue={displayNameOf(user)}
              className="flex cursor-default items-center gap-3 px-3 py-2 text-sm outline-none hover:bg-surface-hover focus:bg-surface-hover selected:bg-accent-soft"
            >
              <Avatar name={displayNameOf(user)} iconId={user.icon} />
              <span className="min-w-0 flex-1 truncate">{displayNameOf(user)}</span>
              <span className="truncate text-xs text-ink-faint">{handleOf(user)}</span>
            </ListBoxItem>
          )}
        </ListBox>
      )}
      <p className="text-xs text-ink-muted">
        {tooMany
          ? format(m.dms.tooMany, { max: String(max + 1) })
          : format(m.dms.selected, { count: String(selected.size) })}
      </p>
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <Button
          isDisabled={pending || selected.size === 0 || tooMany}
          onPress={() => {
            void confirm();
          }}
          className={primaryButtonClass}
        >
          {pending ? pendingLabel : confirmLabel}
        </Button>
      </div>
    </>
  );
}
