import { normalizeServerUrl } from "@aspen/protocol";
import { inviteOf, type InviteRef } from "@/features/invites/inviteCode";
import { deploymentRoute, routeAddress } from "@/features/qr/routeAddress";

/**
 * Something a QR code of Aspen's, or a link it shares, leads to. `domain` is the deployment a
 * link is of: `null` for the one in use.
 */
export type AspenLink =
  | { kind: "invite"; invite: InviteRef }
  | { kind: "registration"; code: string; domain: string | null }
  | { kind: "deviceLink"; server: string; id: string }
  | { kind: "channel"; domain: string | null; community: string; channel: string }
  | { kind: "dm"; domain: string | null; channel: string };

/** A record's id, as the server makes them. */
const ID = "[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}";
/** A community channel's route, as the digest's links name it. */
const CHANNEL_ROUTE = new RegExp(`^/communities/(${ID})/channels/(${ID})$`);
/** A DM's route, likewise. */
const DM_ROUTE = new RegExp(`^/dms/(${ID})$`);

const REGISTRATION_CODE = /^[A-Za-z0-9]{1,16}$/;
/** A device link's id: 32 random bytes, unpadded base64url, as the server makes them. */
const DEVICE_LINK_ID = /^[A-Za-z0-9_-]{32,128}$/;

/**
 * The route a sign-in code's QR code names: the server it belongs to, which a phone that is not
 * signed in has not chosen yet, and the link's id after `#`, which stays out of every server's
 * log when the code opens the web client.
 */
export function deviceLinkPath(server: string, id: string): string {
  return `/device-link?server=${encodeURIComponent(server)}#${id}`;
}

/** The sign-in code a `/device-link` address names, from its query and fragment. */
export function deviceLinkOf(
  query: string,
  fragment: string,
): { server: string; id: string } | null {
  const params = new URLSearchParams(query);
  const id = fragment !== "" ? fragment : (params.get("link") ?? "");
  const given = params.get("server");
  if (given === null || !DEVICE_LINK_ID.test(id)) {
    return null;
  }
  try {
    return { server: normalizeServerUrl(given), id };
  } catch {
    return null;
  }
}

/**
 * Reads what a scanned QR code or an opened link leads to, in any form a link to the app takes
 * (`routeAddress`: under a web client, an `aspen:` link, an app's own page): a sign-in code, a
 * registration invite, a community invite, or a channel or DM, as the links in mail name them.
 * The link's route is its whole path, and what it leads to is the deployment's whose address
 * it is on: the one in use for `homeHosts`, its own addresses, and for a link on no
 * deployment's address. A sign-in code names its server itself. `null` for anything else, bare
 * invite codes included, since text that merely looks like a code is not one.
 */
export function parseAspenLink(text: string, homeHosts: readonly string[]): AspenLink | null {
  const address = routeAddress(text);
  if (address === null) {
    return null;
  }
  if (address.path === "/device-link") {
    const link = deviceLinkOf(address.search.toString(), address.fragment);
    return link === null ? null : { kind: "deviceLink", ...link };
  }
  const invite = inviteOf(address, homeHosts);
  if (invite !== null) {
    return { kind: "invite", invite };
  }
  const route = deploymentRoute(address, homeHosts);
  if (route === null) {
    return null;
  }
  const { domain, path } = route;
  // Accounts are made at a deployment's own address, never under another's `/at/`.
  if (path === "/register" && !route.named) {
    const code = address.search.get("invite") ?? "";
    return REGISTRATION_CODE.test(code) ? { kind: "registration", code, domain } : null;
  }
  const channel = CHANNEL_ROUTE.exec(path);
  if (channel?.[1] !== undefined && channel[2] !== undefined) {
    return { kind: "channel", domain, community: channel[1], channel: channel[2] };
  }
  const dm = DM_ROUTE.exec(path);
  if (dm?.[1] !== undefined) {
    return { kind: "dm", domain, channel: dm[1] };
  }
  return null;
}
