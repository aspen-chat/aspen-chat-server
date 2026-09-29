import type { User } from "./generated/events";

/**
 * Who a user is across deployments: their home's domain and their id there. A record of one of
 * `domain`'s own users is `domain/id`; a record of another deployment's user names its home
 * (`homeDomain`, `homeId`), whichever deployment it is read from.
 */
export function identityOf(
  user: Pick<User, "id" | "homeDomain" | "homeId">,
  domain: string,
): string {
  if (user.homeDomain != null && user.homeId != null) {
    return `${user.homeDomain}/${user.homeId}`;
  }
  return `${domain}/${user.id}`;
}
