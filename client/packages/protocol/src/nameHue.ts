import type { Role, User } from "./generated/events";

/**
 * The hue a person's name is drawn in, or `undefined` for none. A deployment role's hue
 * (`nameHue`, which the server keeps) shows everywhere and wins; otherwise, within a community,
 * the hue of the highest role they hold there that has one. `roles` are the community's, lowest
 * first, and `held` the ids of those the person holds; outside a community pass neither.
 */
export function nameHueOf(
  user: Pick<User, "nameHue"> | undefined,
  roles: readonly Role[] = [],
  held: readonly string[] = [],
): number | undefined {
  if (user?.nameHue != null) {
    return user.nameHue;
  }
  const holds = new Set(held);
  return roles.findLast((role) => role.hue != null && holds.has(role.id))?.hue ?? undefined;
}

/**
 * The highest role shown apart in the member list that a member holds, or `undefined` for
 * none: the group they are listed under while online. `roles` are lowest first.
 */
export function shownApartRole(
  roles: readonly Role[],
  held: readonly string[] | undefined,
): Role | undefined {
  const holds = new Set(held);
  return roles.findLast((role) => role.hoist && holds.has(role.id));
}
