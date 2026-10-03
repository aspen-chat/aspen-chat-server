import type { User, UserOnlineStatus } from "@aspen/protocol";
import { PaneEdge } from "@/features/layout/ResizablePane";
import { ProhibitIcon } from "@phosphor-icons/react";
import { Button } from "react-aria-components";
import { useBlocked, useMembers } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { BotBadge } from "@/features/users/BotBadge";
import { ProfilePopover } from "@/features/users/ProfileCard";
import { displayNameOf, statusLine } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** The member list as a pane beside the channel, on screens wide enough for one. */
export function MemberList({ communityId }: { communityId: string }) {
  const m = useMessages();
  return (
    // The list scrolls within the landmark, so the pane's edge stays along its whole side.
    <aside
      aria-label={m.membersLabel}
      className="motion-from-end relative flex w-full shrink-0 flex-col border-s border-line bg-surface-raised"
    >
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto px-2 py-3">
        <MemberGroups communityId={communityId} headingLevel={2} />
      </div>
      <PaneEdge />
    </aside>
  );
}

/**
 * Who is in the community, online members first. The server samples the most recently seen
 * members, so in a large community this is the active part of the roster rather than all of it.
 * People the user blocked are marked. Each group is headed at `headingLevel`: a level below
 * whatever titles the list.
 */
export function MemberGroups({
  communityId,
  headingLevel,
}: {
  communityId: string;
  headingLevel: 2 | 3;
}) {
  const m = useMessages();
  const members = useMembers(communityId);
  const online = members.filter((u) => u.onlineStatus !== "offline").sort(byName);
  const offline = members.filter((u) => u.onlineStatus === "offline").sort(byName);
  return (
    <>
      <MemberGroup
        heading={format(m.onlineGroup, { count: String(online.length) })}
        headingLevel={headingLevel}
        users={online}
      />
      <MemberGroup
        heading={format(m.offlineGroup, { count: String(offline.length) })}
        headingLevel={headingLevel}
        users={offline}
      />
    </>
  );
}

function byName(a: User, b: User): number {
  return displayNameOf(a).localeCompare(displayNameOf(b));
}

function MemberGroup({
  heading,
  headingLevel,
  users,
}: {
  heading: string;
  headingLevel: 2 | 3;
  users: readonly User[];
}) {
  if (users.length === 0) {
    return null;
  }
  const Heading = headingLevel === 2 ? "h2" : "h3";
  return (
    <section className="mb-3">
      <Heading className="px-2 pb-1 text-xs font-semibold tracking-wide text-ink-faint uppercase">
        {heading}
      </Heading>
      <ul className="flex flex-col gap-0.5">
        {users.map((user) => (
          <MemberRow key={user.id} user={user} />
        ))}
      </ul>
    </section>
  );
}

function MemberRow({ user }: { user: User }) {
  const m = useMessages();
  const offline = user.onlineStatus === "offline";
  const blocked = useBlocked(user.id);
  const name = displayNameOf(user);
  return (
    <li className={offline ? "opacity-60" : ""}>
      <ProfilePopover user={user}>
        <Button
          aria-label={format(m.profile.show, { name })}
          className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-start outline-none hover:bg-surface-hover pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <span className="relative">
            <Avatar name={name} iconId={user.icon} size="md" />
            <StatusDot status={user.onlineStatus} label={m.status[user.onlineStatus]} />
          </span>
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="flex min-w-0 items-center gap-1.5">
              <span className="truncate text-sm">{name}</span>
              {user.bot && <BotBadge />}
            </span>
            {user.status != null && (
              <span className="truncate text-xs text-ink-muted">{statusLine(user.status)}</span>
            )}
          </span>
          {blocked && (
            <ProhibitIcon
              size={14}
              aria-label={m.blocking.blocked}
              className="shrink-0 text-ink-faint"
            />
          )}
        </Button>
      </ProfilePopover>
    </li>
  );
}

function StatusDot({ status, label }: { status: UserOnlineStatus; label: string }) {
  const colour = status === "online" ? "bg-online" : status === "away" ? "bg-away" : "bg-ink-faint";
  return (
    <span
      role="img"
      aria-label={label}
      className={`absolute -end-0.5 -bottom-0.5 h-2.5 w-2.5 rounded-full ring-2 ring-surface-raised ${colour}`}
    />
  );
}
