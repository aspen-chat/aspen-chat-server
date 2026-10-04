import { NAME_COLORS, nameHueOf, type Role } from "@aspen/protocol";
import { useMemberRoles, usePreference, useRoles, useUser } from "@/api/hooks";
import { hueColor } from "@/theme/nameColors";

/**
 * The colour a person's name is drawn in where `communityId` is (`null` or `undefined` in a DM
 * or anywhere outside a community), as a CSS colour, or `undefined` to draw it as text around
 * it is. A deployment role's colour shows everywhere; a community role's only within its
 * community. Nothing is coloured when this install has name colours turned off.
 */
export function useNameColor(
  userId: string | undefined,
  communityId: string | null | undefined,
): string | undefined {
  const on = usePreference(NAME_COLORS);
  const user = useUser(userId);
  const roles = useRoles(communityId ?? "");
  const held = useMemberRoles(communityId ?? "", userId ?? "");
  if (!on) {
    return undefined;
  }
  const hue = communityId == null ? nameHueOf(user) : nameHueOf(user, roles, held);
  return hue === undefined ? undefined : hueColor(hue);
}

/**
 * The colour a role's name is drawn in, as in a tag of it, or `undefined` for none. A
 * deployment role's is drawn alike.
 */
export function useRoleColor(role: Pick<Role, "hue"> | undefined): string | undefined {
  const on = usePreference(NAME_COLORS);
  return on && role?.hue != null ? hueColor(role.hue) : undefined;
}
