import type { User } from "@aspen/protocol";
import { ChatCircleIcon, ProhibitIcon } from "@phosphor-icons/react";
import { useNavigate } from "@tanstack/react-router";
import { useState, type ReactNode, type RefObject } from "react";
import { Button, Dialog, DialogTrigger, Popover } from "react-aria-components";
import { useBlocked, useMe, useSync } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { dangerButtonClass, secondaryButtonClass } from "@/features/invites/dialog";
import { displayNameOf, statusLine } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A user's profile as a card: who they are, their pronouns, what they are up to, and their
 * bio, with ways to message and to block them when they are someone else. Opens from any
 * control that names the user, such as a message author or a member row.
 */
export function ProfileCard({ user }: { user: User }) {
  const m = useMessages();
  const me = useMe();
  const blocked = useBlocked(user.id);
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
          {blocked && (
            <div className="mt-0.5 flex items-center gap-1 text-xs font-medium text-ink-faint">
              <ProhibitIcon size={12} aria-hidden="true" />
              {m.blocking.blocked}
            </div>
          )}
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
      {me !== null && me.id !== user.id && (
        <>
          {!blocked && <MessageButton userId={user.id} />}
          <BlockControl userId={user.id} name={name} blocked={blocked} />
        </>
      )}
    </div>
  );
}

/**
 * Blocks the user, once the caller has read what that does and confirmed, or lifts a block at
 * once.
 */
function BlockControl({
  userId,
  name,
  blocked,
}: {
  userId: string;
  name: string;
  blocked: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const [confirming, setConfirming] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = (action: Promise<void>) => {
    setPending(true);
    setError(null);
    action.then(
      () => {
        setPending(false);
        setConfirming(false);
      },
      (failure: unknown) => {
        setError(failure instanceof Error ? failure.message : String(failure));
        setPending(false);
      },
    );
  };
  return (
    <div className="flex flex-col gap-2">
      {confirming && !blocked && (
        <div className="flex flex-col gap-1">
          <p className="text-sm font-semibold">{format(m.blocking.blockTitle, { name })}</p>
          <p className="text-xs text-ink-muted">{m.blocking.blockExplained}</p>
        </div>
      )}
      {/* Not disabled while pending: a disabled button drops focus out of the card, which
          then no longer closes on Escape. */}
      <Button
        aria-disabled={pending}
        onPress={() => {
          if (pending) {
            return;
          }
          if (blocked) {
            run(sync.unblockUser(userId));
          } else if (confirming) {
            run(sync.blockUser(userId));
          } else {
            setConfirming(true);
          }
        }}
        className={
          (confirming && !blocked ? dangerButtonClass : secondaryButtonClass) +
          " flex items-center justify-center gap-1.5"
        }
      >
        <ProhibitIcon size={16} aria-hidden="true" />
        {blocked ? m.blocking.unblock : m.blocking.block}
      </Button>
      {error !== null && (
        <p role="alert" className="text-xs text-danger">
          {error}
        </p>
      )}
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
