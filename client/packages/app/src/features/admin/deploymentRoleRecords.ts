import type { DeploymentPermission, DeploymentRole } from "@aspen/protocol";
import { useCallback } from "react";
import { useSync } from "@/api/hooks";
import { useAdminRead } from "@/features/admin/useAdminRead";

/** Every deployment permission, in the order the server lists them. */
export const DEPLOYMENT_PERMISSIONS: readonly DeploymentPermission[] = [
  "viewDashboard",
  "manageRegistrationInvites",
  "manageVoiceServers",
  "manageDeploymentRoles",
  "moderateCommunities",
  "manageBots",
  "manageFederation",
  "reviewReports",
  "manageReportCategories",
  "banUsers",
  "messageAnyUser",
];

/** The deployment's roles and the caller's standing among them, read together. */
export interface DeploymentRoles {
  roles: DeploymentRole[];
  /** The caller's permissions and roles. */
  mine: { permissions: DeploymentPermission[]; roles: string[] };
}

/** Reads the deployment's roles with the caller's own, for the sections that need both. */
export function useDeploymentRoles() {
  const sync = useSync();
  const load = useCallback(async (): Promise<DeploymentRoles> => {
    const [roles, mine] = await Promise.all([
      sync.admin.deploymentRoles(),
      sync.admin.deploymentAccess(),
    ]);
    return { roles, mine };
  }, [sync]);
  return useAdminRead(load);
}

/** The caller's rank among the deployment's roles: their highest role's position, 0 with none. */
export function rankOf(read: DeploymentRoles): number {
  return Math.max(
    0,
    ...read.roles.filter((r) => read.mine.roles.includes(r.id)).map((r) => r.position),
  );
}
