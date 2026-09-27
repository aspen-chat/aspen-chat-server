import type { Channel, User } from "@aspen/protocol";
import { displayNameOf } from "@/features/users/profile";

/** Everyone in a DM or group DM besides the caller, in the order they joined. */
export function otherRecipients(channel: Channel, me: string | null): string[] {
  return channel.recipients.filter((id) => id !== me);
}

/**
 * What to call a DM: the other people's names, in the order they joined. `users` are those
 * people's records where the cache has them; `fallback` names a DM whose people are all gone.
 */
export function dmTitle(
  users: readonly (User | undefined)[],
  unknown: string,
  fallback: string,
): string {
  if (users.length === 0) {
    return fallback;
  }
  return users.map((user) => (user === undefined ? unknown : displayNameOf(user))).join(", ");
}
