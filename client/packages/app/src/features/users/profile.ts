import type { CustomStatus, User } from "@aspen/protocol";

/** What to call a user: their display name when they set one, else their username. */
export function displayNameOf(user: Pick<User, "name" | "displayName">): string {
  const display = user.displayName?.trim() ?? "";
  return display.length > 0 ? display : user.name;
}

/**
 * A user's handle: `@name` for this deployment's own users, and `@name@domain` for a user of
 * another deployment, whose name is their home's.
 */
export function handleOf(user: Pick<User, "name" | "homeDomain">): string {
  return user.homeDomain == null ? `@${user.name}` : `@${user.name}@${user.homeDomain}`;
}

/** A status as one line: the emoji, when there is one, then the text. */
export function statusLine(status: CustomStatus): string {
  return status.emoji == null ? status.text : `${status.emoji} ${status.text}`;
}

/** The editable profile fields, as the form holds them: blank means "not set". */
export interface ProfileForm {
  /** The icon's id, or `null` for none. */
  icon: string | null;
  displayName: string;
  pronouns: string;
  bio: string;
  statusText: string;
  statusEmoji: string | null;
}

/** The form's initial values for a user. */
export function profileForm(user: User): ProfileForm {
  return {
    icon: user.icon ?? null,
    displayName: user.displayName ?? "",
    pronouns: user.pronouns ?? "",
    bio: user.bio ?? "",
    statusText: user.status?.text ?? "",
    statusEmoji: user.status?.emoji ?? null,
  };
}

/** A profile merge patch: every field the form changed, with `null` clearing one. */
export interface ProfilePatch {
  icon?: string | null;
  displayName?: string | null;
  pronouns?: string | null;
  bio?: string | null;
  status?: CustomStatus | null;
}

/**
 * The merge patch that takes `user` to what the form holds, or an empty object when nothing
 * changed. Whitespace-only text counts as blank and clears the field. A status emoji without
 * text is dropped, since a status is its text.
 */
export function profilePatch(user: User, form: ProfileForm): ProfilePatch {
  const patch: ProfilePatch = {};
  if (form.icon !== (user.icon ?? null)) {
    patch.icon = form.icon;
  }
  const displayName = blankToNull(form.displayName);
  if (displayName !== (user.displayName ?? null)) {
    patch.displayName = displayName;
  }
  const pronouns = blankToNull(form.pronouns);
  if (pronouns !== (user.pronouns ?? null)) {
    patch.pronouns = pronouns;
  }
  const bio = blankToNull(form.bio);
  if (bio !== (user.bio ?? null)) {
    patch.bio = bio;
  }
  const text = blankToNull(form.statusText);
  const status: CustomStatus | null =
    text === null ? null : form.statusEmoji === null ? { text } : { text, emoji: form.statusEmoji };
  const current = user.status ?? null;
  if (
    (status === null) !== (current === null) ||
    (status !== null &&
      current !== null &&
      (status.text !== current.text || (status.emoji ?? null) !== (current.emoji ?? null)))
  ) {
    patch.status = status;
  }
  return patch;
}

function blankToNull(value: string): string | null {
  const trimmed = value.trim();
  return trimmed.length === 0 ? null : trimmed;
}
