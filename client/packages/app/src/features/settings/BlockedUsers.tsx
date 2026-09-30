import { useState } from "react";
import { Button } from "react-aria-components";
import { useBlockedUsers, useSync, useUser } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { planeClass, secondaryButtonClass } from "@/features/invites/dialog";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";

/** Everyone the user has blocked, each with a way to lift the block. */
export function BlockedUsersSection() {
  const m = useMessages();
  const blocked = useBlockedUsers();
  return (
    <section aria-labelledby="settings-blocked" className={planeClass}>
      <div>
        <h3 id="settings-blocked" className="text-sm font-semibold text-ink-muted">
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
  const user = useUser(userId);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const name = user === undefined ? m.unknownUser : displayNameOf(user);
  return (
    <li className="flex flex-col gap-1">
      <div className="flex items-center gap-2">
        <Avatar name={name} iconId={user?.icon ?? null} size="sm" />
        <span className="min-w-0 flex-1 truncate text-sm">{name}</span>
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
