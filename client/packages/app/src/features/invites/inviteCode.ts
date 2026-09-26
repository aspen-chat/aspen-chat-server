import { router } from "@/router";

/** Server-side invite code rules: alphanumeric, 1 to 16 characters. */
const CODE = /^[A-Za-z0-9]{1,16}$/;

/**
 * The link to share for an invite code. It points at this deployment's own invite route, so on
 * the web it is a plain URL and in a shell it carries the route after a `#`, which the same
 * route handles when pasted back in.
 */
export function inviteLink(code: string): string {
  const href = router.history.createHref(`/invite/${encodeURIComponent(code)}`);
  return new URL(href, window.location.href).toString();
}

/** What an invite link from this deployment looks like, for the join field's placeholder. */
export function inviteLinkExample(): string {
  return `${inviteLink("")}…`;
}

/** Extracts the code from a pasted invite link, or accepts a bare code. `null` if neither. */
export function parseInviteCode(input: string): string | null {
  const trimmed = input.trim();
  if (CODE.test(trimmed)) {
    return trimmed;
  }
  const match = /\/invite\/([A-Za-z0-9]{1,16})(?:[/?#]|$)/.exec(trimmed);
  return match?.[1] ?? null;
}
