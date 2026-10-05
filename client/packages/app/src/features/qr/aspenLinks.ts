import { normalizeServerUrl } from "@aspen/protocol";
import { parseInvite, type InviteRef } from "@/features/invites/inviteCode";

/** Something a QR code of Aspen's, or a link it shares, leads to. */
export type AspenLink =
  | { kind: "invite"; invite: InviteRef }
  | { kind: "registration"; code: string }
  | { kind: "deviceLink"; server: string; id: string }
  | { kind: "channel"; community: string; channel: string }
  | { kind: "dm"; channel: string };

/** A record's id, as the server makes them. */
const ID = "[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}";
/** A community channel's route, as the digest's links name it, under any address. */
const CHANNEL_PATH = new RegExp(`/communities/(${ID})/channels/(${ID})/?(?:[?#].*)?$`);
/** A DM's route, likewise. */
const DM_PATH = new RegExp(`/dms/(${ID})/?(?:[?#].*)?$`);

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
 * Reads what a scanned QR code or an opened link leads to, whatever address it was shared
 * under (a web client, this page, or an `aspen:` link): a sign-in code, a registration invite, a
 * community invite, or a channel or DM, as the links in mail name them. `null` for anything
 * else, bare invite codes included, since text that merely looks like a code is not one.
 */
export function parseAspenLink(text: string): AspenLink | null {
  const trimmed = text.trim();
  const device = /\/device-link\?([^#]*)(?:#(.*))?$/.exec(trimmed);
  if (device !== null) {
    const link = deviceLinkOf(device[1] ?? "", device[2] ?? "");
    return link === null ? null : { kind: "deviceLink", ...link };
  }
  const registration = /\/register\?([^#]*)/.exec(trimmed);
  if (registration !== null) {
    const code = new URLSearchParams(registration[1] ?? "").get("invite") ?? "";
    return REGISTRATION_CODE.test(code) ? { kind: "registration", code } : null;
  }
  const channel = CHANNEL_PATH.exec(trimmed);
  if (channel?.[1] !== undefined && channel[2] !== undefined) {
    return { kind: "channel", community: channel[1], channel: channel[2] };
  }
  const dm = DM_PATH.exec(trimmed);
  if (dm?.[1] !== undefined) {
    return { kind: "dm", channel: dm[1] };
  }
  if (trimmed.includes("/invite/")) {
    const invite = parseInvite(trimmed);
    return invite === null ? null : { kind: "invite", invite };
  }
  return null;
}
