import { deploymentAdministrator } from "../fixtures";
import type { Asked, WorldRoute } from "../reply";

/** The Administration Dashboard's routes, its federation directory among them. */
export function adminRoutes({ request, url, path, admin }: Asked): WorldRoute[] {
  return [
    [
      "GET",
      /^\/users\/@me\/admin$/,
      () => ({
        permissions: [
          "viewDashboard",
          "manageRegistrationInvites",
          "manageVoiceServers",
          "manageDeploymentRoles",
          "manageFederation",
          "manageDeploymentSettings",
        ],
        roles: [deploymentAdministrator],
      }),
    ],
    [
      "GET",
      /^\/admin\/roles$/,
      () => [
        {
          id: deploymentAdministrator,
          name: "Administrator",
          position: 1,
          permissions: [
            "viewDashboard",
            "manageRegistrationInvites",
            "manageVoiceServers",
            "manageDeploymentRoles",
            "manageFederation",
            "manageDeploymentSettings",
          ],
        },
      ],
    ],
    ["GET", /^\/admin\/moderation-log$/, () => []],
    ["GET", /^\/admin\/overview$/, admin.overview],
    ["GET", /^\/admin\/users$/, () => admin.users(url)],
    ["GET", /^\/admin\/communities$/, () => admin.communities(url)],
    ["GET", /^\/admin\/registration-invites$/, admin.invites],
    [
      "POST",
      /^\/admin\/registration-invites$/,
      () => admin.create(request.postDataJSON() as { maxUses?: number; note?: string }),
    ],
    [
      "DELETE",
      /^\/admin\/registration-invites\/[^/]+$/,
      () => admin.revoke(path.split("/").pop() ?? ""),
    ],
    ["GET", /^\/admin\/fleet$/, admin.fleet],
    ["GET", /^\/deployment$/, admin.profile],
    [
      "PATCH",
      /^\/deployment$/,
      () => admin.updateProfile(request.postDataJSON() as { displayName?: string | null }),
    ],
    ["GET", /^\/admin\/federation$/, admin.federation.overview],
    [
      "PATCH",
      /^\/admin\/federation$/,
      () => admin.federation.update(request.postDataJSON() as Record<string, string | boolean>),
    ],
    ["GET", /^\/admin\/settings$/, admin.settings],
    [
      "PATCH",
      /^\/admin\/settings$/,
      () => admin.updateSettings(request.postDataJSON() as Record<string, number | boolean>),
    ],
    ["GET", /^\/admin\/federation\/deployments$/, () => admin.federation.list(url)],
    [
      "POST",
      /^\/admin\/federation\/deployments$/,
      () => admin.federation.add(request.postDataJSON() as { domain: string; note?: string }),
    ],
    [
      "POST",
      /^\/admin\/federation\/deployments\/[^/]+\/contact$/,
      () => admin.federation.contact(path.split("/")[4] ?? ""),
    ],
    [
      "PUT",
      /^\/admin\/federation\/deployments\/[^/]+\/key$/,
      () =>
        admin.federation.accept(
          path.split("/")[4] ?? "",
          (request.postDataJSON() as { publicKey: string }).publicKey,
        ),
    ],
    [
      "PUT",
      /^\/admin\/federation\/deployments\/[^/]+\/lists\/[^/]+$/,
      () => admin.federation.setListed(path.split("/")[4] ?? "", path.split("/")[6] ?? "", true),
    ],
    [
      "DELETE",
      /^\/admin\/federation\/deployments\/[^/]+\/lists\/[^/]+$/,
      () => admin.federation.setListed(path.split("/")[4] ?? "", path.split("/")[6] ?? "", false),
    ],
    [
      "DELETE",
      /^\/admin\/federation\/deployments\/[^/]+$/,
      () => admin.federation.forget(path.split("/")[4] ?? ""),
    ],
    ["GET", /^\/admin\/growth$/, () => admin.growth(url)],
  ];
}
