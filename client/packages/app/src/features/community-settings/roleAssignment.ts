import type { Role } from "@aspen/protocol";
import { useAccess, useCommunity, useMe, useRoles, useStore } from "@/api/hooks";

/** The roles the caller may give a member, and those they may take away. */
export interface AssignableRoles {
  /** Roles they may take away, which takes rank alone. */
  readonly take: readonly Role[];
  /** Roles they may give: those they may take away whose every permission they hold. */
  readonly give: readonly Role[];
}

const NONE: AssignableRoles = { take: [], give: [] };

/**
 * Which of a member's roles the caller may change, as the server decides it: with Assign roles,
 * their own roles, or anyone's but the owner's who ranks below them; and of those, only roles
 * ranked below the caller's highest, never everyone's or a bot's own. Giving a role also takes
 * holding every permission it allows; taking it away takes rank alone. Both are empty when they
 * may change none.
 */
export function useAssignableRoles(communityId: string, userId: string): AssignableRoles {
  const store = useStore();
  const me = useMe();
  const community = useCommunity(communityId);
  const access = useAccess(communityId);
  const roles = useRoles(communityId);
  if (access?.has("assignRoles") !== true) {
    return NONE;
  }
  const self = me?.id === userId;
  const theirs = store.access(communityId, userId);
  const outranked = theirs !== null && access.outranks(theirs.rank);
  if (!self && (community?.owner === userId || !outranked)) {
    return NONE;
  }
  const take = roles.filter(
    (role) => !role.everyone && role.bot == null && access.outranks(role.position),
  );
  return { take, give: take.filter((role) => role.permissions.every((p) => access.has(p))) };
}
