import type { User } from "@aspen/protocol";
import { ChatCircleIcon } from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import { useState, type ReactNode, type RefObject } from "react";
import { Button, Dialog, DialogTrigger, Popover } from "react-aria-components";
import { useMe, useSync } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { displayNameOf, statusLine } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A user's profile as a card: who they are, their pronouns, what they are up to, and their
 * bio, with a way to message them when they are someone else. Opens from any control that
 * names the user, such as a message author or a member row.
 */
export function ProfileCard({ user }: { user: User }) {
  const m = useMessages();
  const me = useMe();
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
      {me !== null && me.id !== user.id && <MessageButton userId={user.id} />}
    </div>
  );
}

/** Opens the caller's DM with the user, making it the first time. */
function MessageButton({ userId }: { userId: string }) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="flex flex-col gap-1">
      <Button
        isDisabled={pending}
        onPress={() => {
          setPending(true);
          setError(null);
          sync.openDm([userId]).then(
            (dm) => {
              void navigate({ to: "/dms/$channelId", params: { channelId: dm.id } });
            },
            (failure: unknown) => {
              setError(failure instanceof Error ? failure.message : String(failure));
              setPending(false);
            },
          );
        }}
        className="flex items-center justify-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-accent-contrast outline-none hover:bg-accent-strong pressed:opacity-80 disabled:opacity-60 focus-visible:ring-2 focus-visible:ring-accent/50"
      >
        <ChatCircleIcon size={16} aria-hidden="true" />
        {m.profile.message}
      </Button>
      {error !== null && (
        <p role="alert" className="text-xs text-danger">
          {error}
        </p>
      )}
    </div>
  );
}

/** Wraps a trigger so pressing it opens the user's profile card beside it. */
export function ProfilePopover({
  user,
  children,
  placement = "bottom start",
  anchorRef,
}: {
  user: User;
  children: ReactNode;
  /** Where the card opens relative to its anchor; `end` puts it beside a list row. */
  placement?: "bottom start" | "end";
  /** What the card is positioned against, when not the trigger itself: the whole row, say. */
  anchorRef?: RefObject<HTMLElement | null>;
}) {
  const m = useMessages();
  return (
    <DialogTrigger>
      {children}
      <Popover
        placement={placement}
        {...(anchorRef === undefined ? {} : { triggerRef: anchorRef })}
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
