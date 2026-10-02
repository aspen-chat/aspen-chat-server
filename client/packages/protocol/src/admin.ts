/**
 * The Administration Dashboard's calls (`AspenSync.admin`). Its reads are queries rather than
 * cached records: each answers what the server says now, and the dashboard holds the answer
 * while it shows it.
 */

import type { components, paths } from "./generated/openapi";
import { type AspenClient, problemOf } from "./http";
import { ApiProblemError } from "./problem";

export type AdminOverview = components["schemas"]["AdminOverview"];
export type AdminUserEntry = components["schemas"]["AdminUserEntry"];
export type AdminCommunityEntry = components["schemas"]["AdminCommunityEntry"];
export type RegistrationInvite = components["schemas"]["RegistrationInvite"];
export type RegistrationInviteRequest = components["schemas"]["RegistrationInviteRequest"];
export type Fleet = components["schemas"]["Fleet"];
export type ApiServerHealth = components["schemas"]["ApiServerHealth"];
export type VoiceServerHealth = components["schemas"]["VoiceServerHealth"];
export type FederationOverview = components["schemas"]["FederationOverview"];
export type FederatedDeployment = components["schemas"]["FederatedDeployment"];
export type FederationList = components["schemas"]["FederationList"];
export type ContactResult = components["schemas"]["ContactResult"];
export type Gate = components["schemas"]["Gate"];

export type UserSort = NonNullable<
  NonNullable<paths["/api/v1/admin/users"]["get"]["parameters"]["query"]>["sort"]
>;
export type CommunitySort = NonNullable<
  NonNullable<paths["/api/v1/admin/communities"]["get"]["parameters"]["query"]>["sort"]
>;
export type Growth = components["schemas"]["Growth"];
export type DeploymentRole = components["schemas"]["DeploymentRole"];
export type DeploymentPermission = components["schemas"]["DeploymentPermission"];
export type ModerationEntry = components["schemas"]["ModerationEntry"];
export type LoggedChannel = components["schemas"]["LoggedChannel"];
export type LoggedMessage = components["schemas"]["LoggedMessage"];
export type FileOfferEntry = components["schemas"]["FileOfferEntry"];
export type GrowthRange = paths["/api/v1/admin/growth"]["get"]["parameters"]["query"]["range"];

/** A page of one of the dashboard's lists. */
export interface AdminListQuery<S extends string> {
  /** Only those whose names contain this, ignoring case. */
  name?: string;
  /** The order; newest first when absent. */
  sort?: S;
  /** How many rows to skip. */
  offset?: number;
  /** How many rows the page holds. */
  limit?: number;
}

function listQuery<S extends string>(
  query: AdminListQuery<S>,
): { "filter[name]"?: string; sort?: S; offset?: number; limit?: number } {
  return {
    ...(query.name === undefined || query.name.trim() === ""
      ? {}
      : { "filter[name]": query.name.trim() }),
    ...(query.sort === undefined ? {} : { sort: query.sort }),
    ...(query.offset === undefined || query.offset === 0 ? {} : { offset: query.offset }),
    ...(query.limit === undefined ? {} : { limit: query.limit }),
  };
}

/** The answer's data, or the Problem the server gave in its place. */
export function adminRead<T>(result: { data?: T; error?: unknown; response: Response }): T {
  if (result.data === undefined) {
    throw new ApiProblemError(problemOf(result.error, result.response));
  }
  return result.data;
}

export class AdminApi {
  readonly #client: AspenClient;

  constructor(client: AspenClient) {
    this.#client = client;
  }

  /** The deployment's totals. */
  async adminOverview(): Promise<AdminOverview> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/overview"));
  }

  /** A page of the deployment's users, searched and sorted. */
  async adminUsers(query: AdminListQuery<UserSort> = {}): Promise<AdminUserEntry[]> {
    return adminRead(
      await this.#client.api.GET("/api/v1/admin/users", { params: { query: listQuery(query) } }),
    );
  }

  /** A page of the deployment's communities, searched and sorted. */
  async adminCommunities(
    query: AdminListQuery<CommunitySort> = {},
  ): Promise<AdminCommunityEntry[]> {
    return adminRead(
      await this.#client.api.GET("/api/v1/admin/communities", {
        params: { query: listQuery(query) },
      }),
    );
  }

  /** The newest registration invites, usable or not. */
  async registrationInvites(): Promise<RegistrationInvite[]> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/registration-invites"));
  }

  /** Makes a registration invite. */
  async createRegistrationInvite(request: RegistrationInviteRequest): Promise<RegistrationInvite> {
    return adminRead(
      await this.#client.api.POST("/api/v1/admin/registration-invites", { body: request }),
    );
  }

  /** Revokes a registration invite; the accounts it made are kept. */
  async revokeRegistrationInvite(code: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/admin/registration-invites/{code}", {
      params: { path: { code } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** This deployment's part in federation: its domain, key, gates, and lists in force. */
  async federation(): Promise<FederationOverview> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/federation"));
  }

  /** A page of the other deployments this one knows, alphabetically, searched by domain. */
  async federatedDeployments(
    query: Omit<AdminListQuery<never>, "sort"> = {},
  ): Promise<FederatedDeployment[]> {
    return adminRead(
      await this.#client.api.GET("/api/v1/admin/federation/deployments", {
        params: { query: listQuery(query) },
      }),
    );
  }

  /** Adds a deployment to the directory, not yet contacted. */
  async addFederatedDeployment(domain: string, note?: string): Promise<FederatedDeployment> {
    return adminRead(
      await this.#client.api.POST("/api/v1/admin/federation/deployments", {
        body: {
          domain,
          ...(note === undefined || note.trim() === "" ? {} : { note: note.trim() }),
        },
      }),
    );
  }

  /** Changes or, with `null`, clears the note kept on a deployment. */
  async setFederatedDeploymentNote(
    domain: string,
    note: string | null,
  ): Promise<FederatedDeployment> {
    return adminRead(
      await this.#client.api.PATCH("/api/v1/admin/federation/deployments/{domain}", {
        params: { path: { domain } },
        body: { note },
      }),
    );
  }

  /** Forgets a deployment: its pinned key and the lists it is on. */
  async removeFederatedDeployment(domain: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/admin/federation/deployments/{domain}", {
      params: { path: { domain } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Reads a deployment's document now, pinning or checking its key. */
  async contactFederatedDeployment(domain: string): Promise<ContactResult> {
    return adminRead(
      await this.#client.api.POST("/api/v1/admin/federation/deployments/{domain}/contact", {
        params: { path: { domain } },
      }),
    );
  }

  /** Accepts the key a deployment offers in place of its pinned one: exactly `publicKey`. */
  async acceptFederatedDeploymentKey(
    domain: string,
    publicKey: string,
  ): Promise<FederatedDeployment> {
    return adminRead(
      await this.#client.api.PUT("/api/v1/admin/federation/deployments/{domain}/key", {
        params: { path: { domain } },
        body: { publicKey },
      }),
    );
  }

  /** Puts a deployment on a list or takes it off. */
  async setFederationListed(domain: string, list: FederationList, listed: boolean): Promise<void> {
    const params = { params: { path: { domain, list } } };
    const result = listed
      ? await this.#client.api.PUT(
          "/api/v1/admin/federation/deployments/{domain}/lists/{list}",
          params,
        )
      : await this.#client.api.DELETE(
          "/api/v1/admin/federation/deployments/{domain}/lists/{list}",
          params,
        );
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** How many users and communities there were at each step of `range`. */
  async adminGrowth(range: GrowthRange): Promise<Growth> {
    return adminRead(
      await this.#client.api.GET("/api/v1/admin/growth", { params: { query: { range } } }),
    );
  }

  /** The health of the deployment's API and voice servers. */
  async fleet(): Promise<Fleet> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/fleet"));
  }

  /** What the caller may do across the deployment, and the roles that give it. */
  async deploymentAccess(): Promise<{ permissions: DeploymentPermission[]; roles: string[] }> {
    return adminRead(await this.#client.api.GET("/api/v1/users/@me/admin"));
  }

  /** The deployment's roles, lowest first, as a query of the moment. */
  async deploymentRoles(): Promise<DeploymentRole[]> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/roles"));
  }

  async createDeploymentRole(
    name: string,
    permissions: readonly DeploymentPermission[],
  ): Promise<DeploymentRole> {
    return adminRead(
      await this.#client.api.POST("/api/v1/admin/roles", {
        body: { name, permissions: [...permissions] },
      }),
    );
  }

  async updateDeploymentRole(
    roleId: string,
    patch: { name?: string; permissions?: readonly DeploymentPermission[] },
  ): Promise<DeploymentRole> {
    return adminRead(
      await this.#client.api.PATCH("/api/v1/admin/roles/{role}", {
        params: { path: { role: roleId } },
        body: {
          ...(patch.name !== undefined ? { name: patch.name } : {}),
          ...(patch.permissions !== undefined ? { permissions: [...patch.permissions] } : {}),
        },
      }),
    );
  }

  async deleteDeploymentRole(roleId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/admin/roles/{role}", {
      params: { path: { role: roleId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Orders the deployment roles below the caller's highest, lowest first. */
  async reorderDeploymentRoles(roleIds: readonly string[]): Promise<DeploymentRole[]> {
    return adminRead(
      await this.#client.api.PUT("/api/v1/admin/role-order", { body: { roles: [...roleIds] } }),
    );
  }

  /** Gives someone a deployment role, or takes it away. */
  async setUserDeploymentRole(userId: string, roleId: string, held: boolean): Promise<void> {
    const params = { params: { path: { user: userId, role: roleId } } };
    const result = held
      ? await this.#client.api.PUT("/api/v1/admin/users/{user}/roles/{role}", params)
      : await this.#client.api.DELETE("/api/v1/admin/users/{user}/roles/{role}", params);
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** A page of the moderation log, newest first. */
  async moderationLog(before?: string): Promise<ModerationEntry[]> {
    return adminRead(
      await this.#client.api.GET("/api/v1/admin/moderation-log", {
        params: { query: before === undefined ? {} : { before } },
      }),
    );
  }

  /** A page of the record of files offered in calls, newest first, optionally one user's. */
  async fileTransferLog(before?: string, user?: string): Promise<FileOfferEntry[]> {
    return adminRead(
      await this.#client.api.GET("/api/v1/admin/file-transfers", {
        params: {
          query: {
            ...(before === undefined ? {} : { before }),
            ...(user === undefined ? {} : { "filter[user]": user }),
          },
        },
      }),
    );
  }
}
