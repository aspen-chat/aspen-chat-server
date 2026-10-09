import type { UserOnlineStatus } from "@aspen/protocol";

const KNOWN: ReadonlySet<string> = new Set<UserOnlineStatus>([
  "online",
  "away",
  "doNotDisturb",
  "offline",
  "invisible",
]);

/**
 * `status` if this client knows it, otherwise offline: a deployment newer than the client may
 * tell of a status it has no shape or name for.
 */
export function knownStatus(status: string): UserOnlineStatus {
  return KNOWN.has(status) ? (status as UserOnlineStatus) : "offline";
}

/** Whether someone shows as about: neither offline nor, as the user sees themself, invisible. */
export function showsConnected(status: UserOnlineStatus): boolean {
  const known = knownStatus(status);
  return known !== "offline" && known !== "invisible";
}
