import type { User } from "@aspen/protocol";
import type { ReactNode } from "react";
import { Dialog, DialogTrigger, Popover } from "react-aria-components";
import { Avatar } from "@/features/communities/Avatar";
import { displayNameOf, statusLine } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A user's profile as a card: who they are, their pronouns, what they are up to, and their
 * bio. Opens from any control that names the user, such as a message author or a member row.
 */
export function ProfileCard({ user }: { user: User }) {
  const m = useMessages();
  const name = displayNameOf(user);
  return (
    <div className="flex w-72 flex-col gap-3 p-4">
      <div className="flex items-center gap-3">
        <Avatar name={name} iconId={user.icon} size="lg" />
        <div className="min-w-0">
          <div className="truncate text-base font-semibold">{name}</div>
          <div className="truncate text-sm text-ink-muted">
            @{user.name}
            {user.pronouns != null && <span> · {user.pronouns}</span>}
          </div>
        </div>
      </div>
      {user.status != null && (
        <p className="text-sm break-words" aria-label={m.profile.statusLabel}>
          {statusLine(user.status)}
        </p>
      )}
      {user.bio != null && (
        <section>
          <h3 className="text-xs font-semibold tracking-wide text-ink-faint uppercase">
            {m.profile.bio}
          </h3>
          <p className="mt-1 text-sm break-words whitespace-pre-wrap">{user.bio}</p>
        </section>
      )}
    </div>
  );
}

/** Wraps a trigger so pressing it opens the user's profile card beside it. */
export function ProfilePopover({ user, children }: { user: User; children: ReactNode }) {
  const m = useMessages();
  return (
    <DialogTrigger>
      {children}
      <Popover
        placement="bottom start"
        className="rounded-lg border border-line bg-surface-raised shadow-lg entering:animate-in exiting:animate-out"
      >
        <Dialog
          aria-label={format(m.profile.cardLabel, { name: displayNameOf(user) })}
          className="outline-none"
        >
          <ProfileCard user={user} />
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}
