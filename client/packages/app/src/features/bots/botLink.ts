import { ALL_PERMISSIONS, type Permission } from "@aspen/protocol";

const KNOWN: ReadonlySet<string> = new Set(ALL_PERMISSIONS);

/**
 * The permissions a bot's link suggests, from its `permissions` parameter: comma separated
 * names, the unknown ones dropped and each kept once, in the order every permission is listed.
 */
export function suggestedPermissions(param: string | undefined): Permission[] {
  const named = new Set((param ?? "").split(",").filter((name) => KNOWN.has(name)));
  return ALL_PERMISSIONS.filter((permission) => named.has(permission));
}

/**
 * The link that opens a bot's add page, suggesting `permissions`. Where the app is served
 * from a web address it is that page's address; in the desktop and mobile shells, whose own
 * addresses mean nothing to anyone else, it is the path to open under any web client's.
 */
export function botAddLink(botId: string, permissions: readonly Permission[]): string {
  const query = permissions.length === 0 ? "" : `?permissions=${permissions.join(",")}`;
  const path = `/bots/${botId}/add${query}`;
  const { protocol, origin } = window.location;
  return protocol === "http:" || protocol === "https:" ? `${origin}${path}` : path;
}
