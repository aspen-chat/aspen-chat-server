import { useState } from "react";
import { Button } from "react-aria-components";
import { useBlockedUsers, useSync } from "@/api/hooks";
import { planeClass, secondaryButtonClass } from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";
import { PersonAvatar, PersonName } from "@/features/users/PersonName";

/** Everyone the user has blocked, each with a way to lift the block. */
export function BlockedUsersSection() {
  const m = useMessages();
  const blocked = useBlockedUsers();
  return (
    <section aria-labelledby="settings-blocked" className={planeClass}>
      <div>
        <h3 id="settings-blocked" className="text-lg font-semibold text-ink-muted">
          {m.blocking.blockedUsers}
        </h3>
        <p className="text-xs text-ink-faint">{m.blocking.blockedUsersHint}</p>
      </div>
      {blocked.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.blocking.noneBlocked}</p>
      ) : (
        <ul aria-labelledby="settings-blocked" className="flex flex-col gap-1">
          {blocked.map((userId) => (
            <BlockedRow key={userId} userId={userId} />
          ))}
        </ul>
      )}
    </section>
  );
}

function BlockedRow({ userId }: { userId: string }) {
  const m = useMessages();
  const sync = useSync();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <li className="flex flex-col gap-1">
      <div className="flex items-center gap-2">
        <PersonAvatar id={userId} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm">
          <PersonName id={userId} />
        </span>
        <Button
          isDisabled={pending}
          onPress={() => {
            setPending(true);
            setError(null);
            sync.unblockUser(userId).catch((failure: unknown) => {
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
    </li>
  );
}
