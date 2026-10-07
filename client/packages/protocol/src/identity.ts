import type { User } from "./generated/events";

/**
 * Who a user is across deployments: their home's domain and their id there. A record of one of
 * `domain`'s own users is `domain/id`. A record of another deployment's user names its home
 * (`homeDomain`, `homeId`), which is believed only when `domain` is `home`, the viewer's own
 * home, which checked the assertion it was made from: any other deployment could name anyone
 * there, and a block of its record would then hide that person everywhere. Its users from
 * elsewhere are `domain/id` like its own.
 */
export function identityOf(
  user: Pick<User, "id" | "homeDomain" | "homeId">,
  domain: string,
  home: string | null,
): string {
  if (user.homeDomain != null && user.homeId != null && home !== null && domain === home) {
    return `${user.homeDomain}/${user.homeId}`;
  }
  return `${domain}/${user.id}`;
}
