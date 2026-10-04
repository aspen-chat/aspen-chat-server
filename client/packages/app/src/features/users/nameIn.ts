import type { User } from "@aspen/protocol";
import { useAccess, useCommunity, useMe, useMemberRoles, useNickname, useStore } from "@/api/hooks";
import { displayNameOf } from "@/features/users/profile";

/**
 * What to call `user` in `communityId`: the nickname they chose there, else their display name or
 * username. Outside a community (`null` or `undefined`) it is always the latter.
 */
export function useNameIn(
  user: Pick<User, "id" | "name" | "displayName"> | undefined,
  communityId: string | null | undefined,
): string | undefined {
  const nickname = useNickname(communityId, user?.id);
  return user === undefined ? undefined : (nickname ?? displayNameOf(user));
}

/**
 * Whether the caller may clear `userId`'s nickname in the community: Manage nicknames, held over
 * someone ranked below them, never the owner. Clearing one's own is the nickname form's.
 */
export function useMayClearNickname(communityId: string, userId: string): boolean {
  const store = useStore();
  const me = useMe();
  const access = useAccess(communityId);
  const community = useCommunity(communityId);
  // Read so the answer follows the member's roles.
  useMemberRoles(communityId, userId);
  if (access === null || me?.id === userId || community?.owner === userId) {
    return false;
  }
  const theirs = store.access(communityId, userId);
  return access.has("manageNicknames") && theirs !== null && access.outranks(theirs.rank);
}
