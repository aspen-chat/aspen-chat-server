import type { FederationList } from "@aspen/protocol";
import type { Messages } from "@/i18n/messages";

/** Whose arrivals or departures a list governs, which way, and whether it lets through or refuses. */
interface ListParts {
  readonly who: "users" | "bots";
  readonly way: "going" | "coming" | "sharedList";
  readonly kind: "allowList" | "blockList";
}

/** Every list the server has, by its whole name, so a list it gains has its parts decided here. */
const LISTS: Readonly<Record<FederationList, ListParts>> = {
  usersEmigrationAllow: { who: "users", way: "going", kind: "allowList" },
  usersEmigrationBlock: { who: "users", way: "going", kind: "blockList" },
  usersImmigrationAllow: { who: "users", way: "coming", kind: "allowList" },
  usersImmigrationBlock: { who: "users", way: "coming", kind: "blockList" },
  usersSharedAllow: { who: "users", way: "sharedList", kind: "allowList" },
  usersSharedBlock: { who: "users", way: "sharedList", kind: "blockList" },
  botsEmigrationAllow: { who: "bots", way: "going", kind: "allowList" },
  botsEmigrationBlock: { who: "bots", way: "going", kind: "blockList" },
  botsImmigrationAllow: { who: "bots", way: "coming", kind: "allowList" },
  botsImmigrationBlock: { who: "bots", way: "coming", kind: "blockList" },
  botsSharedAllow: { who: "bots", way: "sharedList", kind: "allowList" },
  botsSharedBlock: { who: "bots", way: "sharedList", kind: "blockList" },
};

/**
 * Which subject and direction a list governs, as its hint. A list this build does not know (a
 * later server's) is named as the server names it, never guessed at.
 */
export function listHint(m: Messages, list: FederationList): string {
  const parts = (LISTS as Readonly<Partial<Record<string, ListParts>>>)[list];
  if (parts === undefined) {
    return list;
  }
  return `${m.federation[parts.who]} · ${m.federation[parts.way]} · ${m.federation.gate[parts.kind]}`;
}
