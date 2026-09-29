import { router } from "@/router";

/** Server-side invite code rules: alphanumeric, 1 to 16 characters. */
const CODE = /^[A-Za-z0-9]{1,16}$/;

/** An invite as a link or a pasted code names it: its code, and its deployment when named. */
export interface InviteRef {
  readonly code: string;
  /** The deployment the invite belongs to, or `null` when the link does not say. */
  readonly domain: string | null;
}

/**
 * The link to share for an invite of the deployment named `domain` (`null` when that
 * deployment takes no part in federation, so has no domain to name). It opens this client's
 * invite route with `?at=` naming the deployment, so whoever opens it, whatever their home, is
 * taken to the right one. On the web it is a plain URL and in a shell it carries the route
 * after a `#`, which the same route handles when pasted back in.
 */
export function shareableInviteLink(code: string, domain: string | null): string {
  const at = domain === null ? "" : `?at=${encodeURIComponent(domain).replace(/%3A/gi, ":")}`;
  const href = router.history.createHref(`/invite/${encodeURIComponent(code)}${at}`);
  return new URL(href, window.location.href).toString();
}

/** What an invite link from this client looks like, for the join field's placeholder. */
export function inviteLinkExample(): string {
  return `${shareableInviteLink("", null)}…`;
}

function domainOf(encoded: string): string | null {
  try {
    const domain = decodeURIComponent(encoded).trim().toLowerCase();
    return domain === "" ? null : domain;
  } catch {
    return null;
  }
}

/**
 * Reads a pasted invite: a bare code, a shared link (`/invite/{code}`, with `?at=` naming its
 * deployment), or the address of another deployment's invite screen
 * (`/at/{domain}/invite/{code}`). `null` if it is none of them.
 */
export function parseInvite(input: string): InviteRef | null {
  const trimmed = input.trim();
  if (CODE.test(trimmed)) {
    return { code: trimmed, domain: null };
  }
  const abroad = /\/at\/([^/?#]+)\/invite\/([A-Za-z0-9]{1,16})(?:[/?#]|$)/.exec(trimmed);
  if (abroad !== null) {
    const domain = domainOf(abroad[1] ?? "");
    return domain === null ? null : { code: abroad[2] ?? "", domain };
  }
  const match = /\/invite\/([A-Za-z0-9]{1,16})(?:[/?#]|$)/.exec(trimmed);
  if (match === null) {
    return null;
  }
  const at = /[?&]at=([^&#]+)/.exec(trimmed.slice(match.index));
  return { code: match[1] ?? "", domain: at === null ? null : domainOf(at[1] ?? "") };
}
