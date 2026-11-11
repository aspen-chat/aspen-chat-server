import { INVITE_CODE } from "@/features/invites/inviteCodePattern";
import {
  deploymentRoute,
  domainIn,
  routeAddress,
  type RouteAddress,
} from "@/features/qr/routeAddress";
import { shareUrl } from "@/features/qr/shareLinks";

const BARE_CODE = new RegExp(`^${INVITE_CODE}$`);
/** An invite's route, its code captured. */
const INVITE_ROUTE = new RegExp(`^/invite/(${INVITE_CODE})$`);

/** An invite as a link or a pasted code names it: its code, and its deployment when named. */
export interface InviteRef {
  readonly code: string;
  /** The deployment the invite belongs to, or `null` when the link does not say. */
  readonly domain: string | null;
}

/**
 * The route to share for an invite of the deployment named `domain` (`null` when that
 * deployment takes no part in federation, so has no domain to name): this client's invite
 * route with `?at=` naming the deployment, so whoever opens it, whatever their home, is taken to
 * the right one. `shareUrl` makes it a link.
 */
export function invitePath(code: string, domain: string | null): string {
  const at = domain === null ? "" : `?at=${encodeURIComponent(domain).replace(/%3A/gi, ":")}`;
  return `/invite/${encodeURIComponent(code)}${at}`;
}

/** The route to share for a registration invite, which opens the create-account screen. */
export function registrationPath(code: string): string {
  return `/register?invite=${encodeURIComponent(code)}`;
}

/** What an invite link from this client looks like, for the join field's placeholder. */
export function inviteLinkExample(): string {
  return `${shareUrl(invitePath("", null), null)}…`;
}

/** An invite link, read whole. */
interface InviteLink {
  readonly code: string;
  /** The deployment the link is on, `null` for the one in use. */
  readonly domain: string | null;
  /** Whether the link itself names the deployment, by `?at=` or `/at/{domain}`. */
  readonly named: boolean;
}

/**
 * The invite `address` is the route of: `/invite/{code}`, whose `?at=` names its deployment
 * whatever address the link is on, since any client may have made it, or
 * `/at/{domain}/invite/{code}`, another deployment's invite screen. A link that names none is
 * an invite of the deployment whose address it is on. `null` for any other route.
 */
function inviteAt(address: RouteAddress, homeHosts: readonly string[]): InviteLink | null {
  const route = deploymentRoute(address, homeHosts);
  const code = route === null ? undefined : INVITE_ROUTE.exec(route.path)?.[1];
  if (route === null || code === undefined) {
    return null;
  }
  const at = route.named ? null : address.search.get("at");
  const named = at === null ? null : domainIn(at);
  return named !== null
    ? { code, domain: named, named: true }
    : { code, domain: route.domain, named: route.named };
}

/**
 * Reads a pasted invite: a bare code, which is the deployment in use's; a shared link
 * (`/invite/{code}`, with `?at=` naming its deployment); or the address of another deployment's
 * invite screen (`/at/{domain}/invite/{code}`). A link is read whole (`routeAddress`), and one
 * that names no deployment belongs to the one whose address it is on: the deployment in use
 * for `homeHosts`, its own addresses, and otherwise the deployment at the link's host. `null`
 * if it is none of them.
 */
export function parseInvite(input: string, homeHosts: readonly string[]): InviteRef | null {
  const trimmed = input.trim();
  if (BARE_CODE.test(trimmed)) {
    return { code: trimmed, domain: null };
  }
  return inviteOf(routeAddress(trimmed), homeHosts);
}

/** The invite a link's route is, as `parseInvite` reads one. */
export function inviteOf(
  address: RouteAddress | null,
  homeHosts: readonly string[],
): InviteRef | null {
  const link = address === null ? null : inviteAt(address, homeHosts);
  return link === null ? null : { code: link.code, domain: link.domain };
}

/**
 * The invite a link in a message is when the link itself names the invite's deployment, as
 * every invite link an Aspen client shares does. A link that only looks like one by its path
 * (`https://elsewhere.example/invite/abc`) is not taken for one there: most such links are
 * other sites' own.
 */
export function namedInvite(href: string): (InviteRef & { domain: string }) | null {
  const address = routeAddress(href);
  const link = address === null ? null : inviteAt(address, []);
  return link === null || !link.named || link.domain === null
    ? null
    : { code: link.code, domain: link.domain };
}
