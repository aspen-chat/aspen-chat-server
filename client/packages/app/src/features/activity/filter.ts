import type { ActivityFilter, PreferenceDefinition } from "@aspen/protocol";

/**
 * What the activity feed leaves out, as this device remembers it: whole deployments, single
 * communities, and a deployment's DMs, each by `hiddenKey`. Everything else shows, so a
 * community joined or a deployment added later shows until it too is left out.
 */
export const ACTIVITY_HIDDEN: PreferenceDefinition<readonly string[]> = {
  key: "activity.hidden",
  scope: "device",
  fallback: [],
  parse: (raw) =>
    Array.isArray(raw) && raw.every((key) => typeof key === "string") ? raw : undefined,
};

/** Whether the activity feed shows only what the reader has not read, as this device remembers. */
export const ACTIVITY_UNREAD_ONLY: PreferenceDefinition<boolean> = {
  key: "activity.unreadOnly",
  scope: "device",
  fallback: false,
  parse: (raw) => (typeof raw === "boolean" ? raw : undefined),
};

/** What may be left out of the feed: a deployment (`null` the home), or one of its communities or its DMs. */
export type FeedPart =
  | { kind: "deployment"; domain: string | null }
  | { kind: "community"; domain: string | null; community: string }
  | { kind: "dms"; domain: string | null };

/** The key a part is remembered by in `ACTIVITY_HIDDEN`. */
export function hiddenKey(part: FeedPart): string {
  const domain = part.domain ?? "";
  switch (part.kind) {
    case "deployment":
      return `deployment:${domain}`;
    case "community":
      return `community:${domain}:${part.community}`;
    case "dms":
      return `dms:${domain}`;
  }
}

/**
 * A community id no one belongs to, standing for "none of them" where a read must name at
 * least one community to leave every other out.
 */
export const NO_COMMUNITY = "00000000-0000-0000-0000-000000000000";

/**
 * What to ask a deployment for, given what is hidden and the communities the reader belongs to
 * there; `null` when nothing of it shows.
 */
export function readFilter(
  hidden: ReadonlySet<string>,
  domain: string | null,
  communities: readonly string[],
): Pick<ActivityFilter, "communities" | "dms"> | null {
  if (hidden.has(hiddenKey({ kind: "deployment", domain }))) {
    return null;
  }
  const dms = !hidden.has(hiddenKey({ kind: "dms", domain }));
  const shown = communities.filter(
    (community) => !hidden.has(hiddenKey({ kind: "community", domain, community })),
  );
  if (shown.length === communities.length) {
    return { dms };
  }
  if (shown.length === 0 && !dms) {
    return null;
  }
  return { dms, communities: shown.length === 0 ? [NO_COMMUNITY] : shown };
}

/** Whether a message of `community` (`null` for a DM) on `domain` shows in the feed. */
export function shows(
  hidden: ReadonlySet<string>,
  domain: string | null,
  community: string | null,
): boolean {
  return (
    !hidden.has(hiddenKey({ kind: "deployment", domain })) &&
    !hidden.has(
      hiddenKey(
        community === null ? { kind: "dms", domain } : { kind: "community", domain, community },
      ),
    )
  );
}
