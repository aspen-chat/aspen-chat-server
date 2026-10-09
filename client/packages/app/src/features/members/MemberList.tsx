import { shownApartRole, type Role, type User } from "@aspen/protocol";
import { PaneEdge } from "@/features/layout/ResizablePane";
import { ProhibitIcon } from "@phosphor-icons/react";
import { Button } from "react-aria-components";
import { useBlocked, useMembers, useNicknames, useRolesOfMembers } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { BotBadge } from "@/features/users/BotBadge";
import { ProfilePopover } from "@/features/users/ProfileCard";
import { useNameColor } from "@/features/users/nameColor";
import { useNameIn } from "@/features/users/nameIn";
import { StatusDot } from "@/features/users/PresenceMark";
import { knownStatus, showsConnected } from "@/features/users/presenceStatus";
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
 * Who is in the community, online members first: those holding a role shown apart under the
 * highest such role, highest first, then the rest, then everyone offline whatever their roles.
 * The server samples the members, connected ones and holders of roles shown apart first, so in
 * a large community this is the active part of the roster rather than all of it. People the
 * user blocked are marked. Each group is headed at `headingLevel`: a level below whatever titles
 * the list.
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
  const { roles, of } = useRolesOfMembers(communityId);
  const nicknames = useNicknames(communityId);
  const byName = (a: User, b: User): number =>
    (nicknames.get(a.id) ?? displayNameOf(a)).localeCompare(
      nicknames.get(b.id) ?? displayNameOf(b),
    );
  const apart = new Map<string, { role: Role; users: User[] }>();
  const online: User[] = [];
  const offline: User[] = [];
  for (const user of members) {
    if (!showsConnected(user.onlineStatus)) {
      offline.push(user);
      continue;
    }
    const role = shownApartRole(roles, of(user.id));
    if (role === undefined) {
      online.push(user);
    } else {
      const group = apart.get(role.id) ?? { role, users: [] };
      group.users.push(user);
      apart.set(role.id, group);
    }
  }
  const groups = Array.from(apart.values()).sort((a, b) => b.role.position - a.role.position);
  return (
    <>
      {groups.map(({ role, users }) => (
        <MemberGroup
          key={role.id}
          heading={format(m.roleGroup, { role: role.name, count: String(users.length) })}
          headingLevel={headingLevel}
          users={users.sort(byName)}
          communityId={communityId}
        />
      ))}
      <MemberGroup
        heading={format(m.onlineGroup, { count: String(online.length) })}
        headingLevel={headingLevel}
        users={online.sort(byName)}
        communityId={communityId}
      />
      <MemberGroup
        heading={format(m.offlineGroup, { count: String(offline.length) })}
        headingLevel={headingLevel}
        users={offline.sort(byName)}
        communityId={communityId}
      />
    </>
  );
}

function MemberGroup({
  heading,
  headingLevel,
  users,
  communityId,
}: {
  heading: string;
  headingLevel: 2 | 3;
  users: readonly User[];
  communityId: string;
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
          <MemberRow key={user.id} user={user} communityId={communityId} />
        ))}
      </ul>
    </section>
  );
}

function MemberRow({ user, communityId }: { user: User; communityId: string }) {
  const m = useMessages();
  const offline = !showsConnected(user.onlineStatus);
  const blocked = useBlocked(user.id);
  const name = useNameIn(user, communityId) ?? displayNameOf(user);
  // An offline row is dimmed, and a dimmed colour would no longer read against the list, so an
  // offline name keeps the plain ink, which reads dimmed.
  const nameColor = useNameColor(user.id, communityId);
  return (
    <li className={offline ? "opacity-60" : ""}>
      <ProfilePopover user={user}>
        <Button
          aria-label={format(m.profile.show, { name })}
          className="flex w-full items-center gap-2 rounded-md px-2 py-1 text-start outline-none hover:bg-surface-hover pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <span className="relative">
            <Avatar name={name} iconId={user.icon} size="md" />
            <StatusDot
              status={user.onlineStatus}
              label={m.status[knownStatus(user.onlineStatus)]}
            />
          </span>
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="flex min-w-0 items-center gap-1.5">
              <span className="truncate text-sm" style={{ color: offline ? undefined : nameColor }}>
                {name}
              </span>
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
