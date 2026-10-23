import type { User } from "@aspen/protocol";
import { ProhibitIcon } from "@phosphor-icons/react";
import { useMemo, useState } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  GridList,
  GridListItem,
  Input,
  Label,
  ListLayout,
  Modal,
  ModalOverlay,
  SearchField,
  Virtualizer,
  useFilter,
} from "react-aria-components";
import { useBlockedUsers, useSync, useUsers } from "@/api/hooks";
import { fieldClass, inputClass, labelClass } from "@/features/auth/styles";
import {
  dialogClass,
  overlayClass,
  planeClass,
  planeSurfaceClass,
  planesModalClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { PersonAvatar, PersonName } from "@/features/users/PersonName";
import { displayNameOf, handleOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** A row's height before it is measured: its picture, name, and button on one line. */
const ROW_HEIGHT = 44;

/**
 * The people the user has blocked, in Settings: how many, and a button opening the list of
 * them (`BlockedUsersDialog`), which may run to the thousands the server allows.
 */
export function BlockedUsersSection() {
  const m = useMessages();
  const count = useBlockedUsers().length;
  return (
    <section aria-labelledby="settings-blocked" className={planeClass}>
      <div>
        <h3 id="settings-blocked" className="text-lg font-semibold text-ink-muted">
          {m.blocking.blockedUsers}
        </h3>
        <p className="text-xs text-ink-faint">{m.blocking.blockedUsersHint}</p>
      </div>
      {count === 0 && <p className="text-sm text-ink-muted">{m.blocking.noneBlocked}</p>}
      <BlockedUsersDialog count={count} />
    </section>
  );
}

/**
 * Everyone the user has blocked, each with a way to lift the block, in a modal of its own: found
 * by their name or handle, and drawn only as far as the list is scrolled, so a list of
 * thousands opens as quickly as one of a few. Its button is offered while there is anyone to
 * list; the modal stays open as the last block is lifted, and says so.
 */
function BlockedUsersDialog({ count }: { count: number }) {
  const m = useMessages();
  return (
    <DialogTrigger>
      {count > 0 && (
        <Button className={secondaryButtonClass + " flex items-center gap-1.5 self-start"}>
          <ProhibitIcon size={16} aria-hidden="true" />
          {format(m.blocking.showBlocked, { count })}
        </Button>
      )}
      <ModalOverlay isDismissable className={overlayClass}>
        <Modal className={planesModalClass}>
          <Dialog className={dialogClass}>
            <DialogHeading>{m.blocking.blockedUsers}</DialogHeading>
            <BlockedUsersList />
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

function BlockedUsersList() {
  const m = useMessages();
  const blocked = useBlockedUsers();
  const users = useUsers(blocked);
  const [query, setQuery] = useState("");
  const { contains } = useFilter({ sensitivity: "base" });
  const shown = useMemo(() => {
    const rows = blocked.map((id, i) => ({ id, user: users[i] }));
    const wanted = query.trim();
    return wanted === ""
      ? rows
      : rows.filter(({ user }) => user !== undefined && matches(user, wanted, contains));
  }, [blocked, users, query, contains]);
  return (
    <>
      <p className="text-sm text-ink-muted">{m.blocking.blockedUsersHint}</p>
      <SearchField value={query} onChange={setQuery} className={fieldClass}>
        <Label className={labelClass}>{m.blocking.search}</Label>
        <Input className={inputClass} />
      </SearchField>
      {blocked.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.blocking.noneBlocked}</p>
      ) : shown.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.blocking.noneMatch}</p>
      ) : (
        // The plane's padding is outside the list, which sizes its rows to its own width.
        <div className={planeSurfaceClass}>
          <Virtualizer layout={ListLayout} layoutOptions={{ estimatedRowHeight: ROW_HEIGHT }}>
            <GridList
              aria-label={m.blocking.blockedUsers}
              items={shown}
              className="max-h-[min(24rem,50dvh)] overflow-y-auto outline-none"
            >
              {({ id, user }) => <BlockedRow id={id} user={user} />}
            </GridList>
          </Virtualizer>
        </div>
      )}
    </>
  );
}

/** Whether `query` is in the user's name or handle, as `contains` compares them. */
function matches(user: User, query: string, contains: (text: string, part: string) => boolean) {
  return contains(displayNameOf(user), query) || contains(handleOf(user), query);
}

function BlockedRow({ id, user }: { id: string; user: User | undefined }) {
  const m = useMessages();
  const sync = useSync();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <GridListItem
      id={id}
      textValue={user === undefined ? id : displayNameOf(user)}
      className="flex flex-col gap-1 rounded-md px-1 py-1.5 outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      <div className="flex items-center gap-2">
        <PersonAvatar id={id} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm">
          <PersonName id={id} />
        </span>
        <Button
          isDisabled={pending}
          onPress={() => {
            setPending(true);
            setError(null);
            sync.unblockUser(id).catch((failure: unknown) => {
              setError(failure instanceof Error ? failure.message : String(failure));
              setPending(false);
            });
          }}
          className={secondaryButtonClass}
        >
          {m.blocking.unblock}
        </Button>
      </div>
      {error !== null && (
        <p role="alert" className="text-xs text-danger">
          {error}
        </p>
      )}
    </GridListItem>
  );
}
