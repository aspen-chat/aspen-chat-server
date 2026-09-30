import type { Role } from "@aspen/protocol";
import { useAccess, useCommunity, useMe, useRoles, useStore } from "@/api/hooks";

/**
 * Which of a member's roles the caller may change, as the server decides it: with Assign roles,
 * their own roles, or anyone's but the owner's who ranks below them; and of those, only roles
 * ranked below the caller's highest, never everyone's or a bot's own. Empty when they may
 * change none.
 */
export function useAssignableRoles(communityId: string, userId: string): readonly Role[] {
  const store = useStore();
  const me = useMe();
  const community = useCommunity(communityId);
  const access = useAccess(communityId);
  const roles = useRoles(communityId);
  if (access?.has("assignRoles") !== true) {
    return [];
  }
  const self = me?.id === userId;
  const theirs = store.access(communityId, userId);
  const outranked = theirs !== null && access.outranks(theirs.rank);
  if (!self && (community?.owner === userId || !outranked)) {
    return [];
  }
  return roles.filter(
    (role) => !role.everyone && role.bot == null && access.outranks(role.position),
  );
}
