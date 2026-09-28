import { ApiProblemError, type User } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { useAccess, useMembers, useRoles, useStore, useSync } from "@/api/hooks";

/** How long typing pauses before a search is sent. */
const SEARCH_DELAY_MS = 300;

/** What a member search shows. */
export interface MemberSearch {
  /** The members found, or the member sample while nothing is typed. */
  members: readonly User[];
  searching: boolean;
  error: string | null;
  /**
   * Whether the caller may search every member. The server allows it to those who act on
   * members; everyone else has the sample, which in a small community is everyone.
   */
  canSearch: boolean;
}

/**
 * The members of a community whose name contains `query`, searched on the server once typing
 * pauses, or the member sample while nothing is typed or the caller may not search. The people
 * found follow their events like any cached record, and leave the results when they leave the
 * community.
 */
export function useMemberSearch(communityId: string, query: string): MemberSearch {
  const sync = useSync();
  const store = useStore();
  const sample = useMembers(communityId);
  // Memberships and roles share this topic, so someone found who then leaves or is removed
  // drops out of the results with their membership's event.
  useRoles(communityId);
  const access = useAccess(communityId);
  const canSearch =
    access !== null &&
    (access.owner ||
      access.moderator ||
      access.has("assignRoles") ||
      access.has("removeMembers") ||
      access.has("manageChannels") ||
      access.has("manageCategories"));
  const text = query.trim();
  const [found, setFound] = useState<{ text: string; members: readonly User[] } | null>(null);
  const [error, setError] = useState<{ text: string; message: string } | null>(null);
  useEffect(() => {
    if (!canSearch || text === "") {
      return;
    }
    let current = true;
    const timer = setTimeout(() => {
      sync.searchMembers(communityId, text).then(
        (members) => {
          if (current) {
            setFound({ text, members });
            setError(null);
          }
        },
        (e: unknown) => {
          if (current) {
            setError({ text, message: e instanceof ApiProblemError ? e.message : String(e) });
          }
        },
      );
    }, SEARCH_DELAY_MS);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [sync, communityId, text, canSearch]);
  if (!canSearch || text === "") {
    return { members: sample, searching: false, error: null, canSearch };
  }
  return {
    members: (found?.members ?? []).filter(
      (u) => store.memberRoles(communityId, u.id) !== undefined,
    ),
    searching: found?.text !== text && error?.text !== text,
    error: error?.text === text ? error.message : null,
    canSearch,
  };
}
