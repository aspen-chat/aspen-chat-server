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
export type UnregisteredVoiceServer = components["schemas"]["UnregisteredVoiceServer"];
export type FederationOverview = components["schemas"]["FederationOverview"];
export type FederatedDeployment = components["schemas"]["FederatedDeployment"];
export type FederationList = components["schemas"]["FederationList"];
export type ContactResult = components["schemas"]["ContactResult"];
export type Gate = components["schemas"]["Gate"];
export type DeploymentProfileUpdateRequest =
  components["schemas"]["DeploymentProfileUpdateRequest"];
export type DeploymentSettings = components["schemas"]["DeploymentSettings"];
export type DeploymentSettingsUpdateRequest =
  components["schemas"]["DeploymentSettingsUpdateRequest"];
export type FederationUpdateRequest = components["schemas"]["FederationUpdateRequest"];
export type NewsletterPost = components["schemas"]["NewsletterPostRecord"];
export type NewsletterPostCreateRequest = components["schemas"]["NewsletterPostCreateRequest"];
export type NewsletterPostUpdateRequest = components["schemas"]["NewsletterPostUpdateRequest"];

export type UserSort = NonNullable<
  NonNullable<paths["/api/v1/admin/users"]["get"]["parameters"]["query"]>["sort"]
>;
export type CommunitySort = NonNullable<
  NonNullable<paths["/api/v1/admin/communities"]["get"]["parameters"]["query"]>["sort"]
>;
export type Growth = components["schemas"]["Growth"];
export type DeploymentRole = components["schemas"]["DeploymentRole"];
export type DeploymentPermission = components["schemas"]["DeploymentPermission"];
export type AdminAccess = components["schemas"]["AdminAccess"];
export type ModerationEntry = components["schemas"]["ModerationEntry"];
export type LoggedChannel = components["schemas"]["LoggedChannel"];
export type LoggedMessage = components["schemas"]["LoggedMessage"];
export type FileOfferEntry = components["schemas"]["FileOfferEntry"];
export type JobsOverview = components["schemas"]["JobsOverview"];
export type JobEntry = components["schemas"]["JobEntry"];
export type JobClass = components["schemas"]["JobClass"];
export type GrowthRange = paths["/api/v1/admin/growth"]["get"]["parameters"]["query"]["range"];
export type UserBanRequest = components["schemas"]["UserBanRequest"];
export type UserBanOutcome = components["schemas"]["UserBanOutcome"];
export type ReportCase = components["schemas"]["ReportCase"];
export type ReportCaseList = components["schemas"]["ReportCaseList"];
export type ReportCounts = components["schemas"]["ReportCounts"];
export type ReportStatus = components["schemas"]["ReportStatus"];
export type ReportContext = components["schemas"]["ReportContext"];
export type ReportResolutionRequest = components["schemas"]["ReportResolutionRequest"];
export type ReportCategory = components["schemas"]["ReportCategory"];
export type ReviewedMessage = components["schemas"]["ReviewedMessage"];
export type ProfileAspect = components["schemas"]["ProfileAspect"];
export type ProfileSnapshot = components["schemas"]["ProfileSnapshot"];
export type Report = components["schemas"]["Report"];
export type Resolution = components["schemas"]["Resolution"];
export type AdminPlugin = components["schemas"]["AdminPlugin"];
export type AdminPluginUpdateRequest = components["schemas"]["AdminPluginUpdateRequest"];

/** A page of one of the dashboard's lists. */
export interface AdminListQuery<S extends string> {
  /** Only those whose names contain this, ignoring case. */
  name?: string;
  /** Only those banned from the deployment now (the user list). */
  banned?: boolean;
  /** The order; newest first when absent. */
  sort?: S;
  offset?: number;
  limit?: number;
}

function listQuery<S extends string>(
  query: AdminListQuery<S>,
): {
  "filter[name]"?: string;
  "filter[banned]"?: boolean;
  sort?: S;
  offset?: number;
  limit?: number;
} {
  return {
    ...(query.banned === true ? { "filter[banned]": true } : {}),
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

  /**
   * Bans someone from the deployment: their sign-ins end, and they cannot sign in until the ban
   * ends or is lifted. Takes Ban users.
   */
  async banUser(userId: string, request: UserBanRequest): Promise<UserBanOutcome> {
    return adminRead(
      await this.#client.api.PUT("/api/v1/admin/users/{user}/ban", {
        params: { path: { user: userId } },
        body: request,
      }),
    );
  }

  /** Lifts a ban from the deployment. Takes Ban users. */
  async liftUserBan(userId: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/admin/users/{user}/ban", {
      params: { path: { user: userId } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** A page of the report cases in one state. Takes Review reports. */
  async reportCases(
    status: ReportStatus,
    page: { offset?: number; limit?: number } = {},
  ): Promise<ReportCaseList> {
    return adminRead(
      await this.#client.api.GET("/api/v1/admin/reports", {
        params: {
          query: {
            "filter[status]": status,
            ...(page.offset === undefined || page.offset === 0 ? {} : { offset: page.offset }),
            ...(page.limit === undefined ? {} : { limit: page.limit }),
          },
        },
      }),
    );
  }

  /** One report case, in the shape of a list of cases. */
  async reportCase(caseId: string): Promise<ReportCaseList> {
    return adminRead(
      await this.#client.api.GET("/api/v1/admin/reports/{case}", {
        params: { path: { case: caseId } },
      }),
    );
  }

  /** How many report cases are open, and how many dismissed. */
  async reportCounts(): Promise<ReportCounts> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/report-counts"));
  }

  /** The messages around a case's reported message; around it, or before or after one. */
  async reportContext(
    caseId: string,
    anchor: { before?: string; after?: string } = {},
  ): Promise<ReportContext> {
    return adminRead(
      await this.#client.api.GET("/api/v1/admin/reports/{case}/context", {
        params: { path: { case: caseId }, query: anchor },
      }),
    );
  }

  /** Resolves an open case with the actions given. */
  async resolveReportCase(
    caseId: string,
    request: ReportResolutionRequest,
  ): Promise<ReportCaseList> {
    return adminRead(
      await this.#client.api.POST("/api/v1/admin/reports/{case}/resolution", {
        params: { path: { case: caseId } },
        body: request,
      }),
    );
  }

  /** Dismisses an open case, or restores a dismissed one to review. */
  async setReportCaseDismissed(caseId: string, dismissed: boolean): Promise<void> {
    const params = { params: { path: { case: caseId } } };
    const result = dismissed
      ? await this.#client.api.PUT("/api/v1/admin/reports/{case}/dismissal", params)
      : await this.#client.api.DELETE("/api/v1/admin/reports/{case}/dismissal", params);
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Every report category, hidden ones included, in the order they are offered. */
  async allReportCategories(): Promise<ReportCategory[]> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/report-categories"));
  }

  /** Adds a category of the deployment's own. */
  async createReportCategory(name: string, description: string | null): Promise<ReportCategory> {
    return adminRead(
      await this.#client.api.POST("/api/v1/admin/report-categories", {
        body: { name, ...(description === null ? {} : { description }) },
      }),
    );
  }

  /** Renames, describes, hides, or shows a category, as a merge patch. */
  async updateReportCategory(
    categoryId: string,
    patch: { name?: string; description?: string | null; hidden?: boolean },
  ): Promise<ReportCategory> {
    return adminRead(
      await this.#client.api.PATCH("/api/v1/admin/report-categories/{category}", {
        params: { path: { category: categoryId } },
        body: patch,
      }),
    );
  }

  /** Puts the deployment's own categories in order. */
  async orderReportCategories(categoryIds: readonly string[]): Promise<void> {
    const result = await this.#client.api.PUT("/api/v1/admin/report-category-order", {
      body: { categories: [...categoryIds] },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** The newest registration invites, usable or not. */
  async registrationInvites(): Promise<RegistrationInvite[]> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/registration-invites"));
  }

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

  /**
   * Changes how the deployment presents itself: its display name and icon, each cleared with
   * `null`. Answers with the profile as it now is.
   */
  async updateDeploymentProfile(
    change: DeploymentProfileUpdateRequest,
  ): Promise<components["schemas"]["DeploymentProfile"]> {
    return adminRead(await this.#client.api.PATCH("/api/v1/deployment", { body: change }));
  }

  /** The policies the deployment's administrators set. */
  async deploymentSettings(): Promise<DeploymentSettings> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/settings"));
  }

  /**
   * Changes the deployment's policies, for every server at once. Answers with them as they now
   * are.
   */
  async updateDeploymentSettings(
    change: DeploymentSettingsUpdateRequest,
  ): Promise<DeploymentSettings> {
    return adminRead(await this.#client.api.PATCH("/api/v1/admin/settings", { body: change }));
  }

  /** Every newsletter post, the newest first. */
  async newsletterPosts(): Promise<NewsletterPost[]> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/newsletter/posts"));
  }

  /** Writes a newsletter draft. */
  async createNewsletterPost(post: NewsletterPostCreateRequest): Promise<NewsletterPost> {
    return adminRead(await this.#client.api.POST("/api/v1/admin/newsletter/posts", { body: post }));
  }

  /** Changes a newsletter draft; a sent post is refused (`conflict`). */
  async updateNewsletterPost(
    post: string,
    change: NewsletterPostUpdateRequest,
  ): Promise<NewsletterPost> {
    return adminRead(
      await this.#client.api.PATCH("/api/v1/admin/newsletter/posts/{post}", {
        params: { path: { post } },
        body: change,
      }),
    );
  }

  /** Deletes a newsletter draft. */
  async deleteNewsletterPost(post: string): Promise<void> {
    const result = await this.#client.api.DELETE("/api/v1/admin/newsletter/posts/{post}", {
      params: { path: { post } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Mails the post to the caller alone, at their verified address. */
  async testNewsletterPost(post: string): Promise<void> {
    const result = await this.#client.api.POST("/api/v1/admin/newsletter/posts/{post}/test", {
      params: { path: { post } },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }

  /** Sends the post to every subscriber, once; it is fixed from then on. */
  async sendNewsletterPost(post: string): Promise<NewsletterPost> {
    return adminRead(
      await this.#client.api.POST("/api/v1/admin/newsletter/posts/{post}/sending", {
        params: { path: { post } },
      }),
    );
  }

  /** This deployment's part in federation: its domain, key, gates, and lists in force. */
  async federation(): Promise<FederationOverview> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/federation"));
  }

  /**
   * Changes the gates, for every server at once; users of other deployments a closed gate no
   * longer admits are signed out. Answers with this deployment's part in federation as it now is.
   */
  async updateFederation(change: FederationUpdateRequest): Promise<FederationOverview> {
    return adminRead(await this.#client.api.PATCH("/api/v1/admin/federation", { body: change }));
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
  async deploymentAccess(): Promise<AdminAccess> {
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

  /** Changes a deployment role as a merge patch; a `hue` of `null` takes its colour away. */
  async updateDeploymentRole(
    roleId: string,
    patch: {
      name?: string;
      permissions?: readonly DeploymentPermission[];
      hue?: number | null;
    },
  ): Promise<DeploymentRole> {
    return adminRead(
      await this.#client.api.PATCH("/api/v1/admin/roles/{role}", {
        params: { path: { role: roleId } },
        body: {
          ...(patch.name !== undefined ? { name: patch.name } : {}),
          ...(patch.permissions !== undefined ? { permissions: [...patch.permissions] } : {}),
          ...(patch.hue !== undefined ? { hue: patch.hue } : {}),
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

  /** What the deployment's background jobs are doing now: a preview of at most 100. */
  async jobs(): Promise<JobsOverview> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/jobs"));
  }

  /** Every installed plugin, on or off, in the order they decide messages in. */
  async plugins(): Promise<AdminPlugin[]> {
    return adminRead(await this.#client.api.GET("/api/v1/admin/plugins"));
  }

  /** Turns an installed plugin on or off, changes its mode, or configures it. */
  async updatePlugin(pluginId: string, patch: AdminPluginUpdateRequest): Promise<AdminPlugin> {
    return adminRead(
      await this.#client.api.PATCH("/api/v1/admin/plugins/{plugin}", {
        params: { path: { plugin: pluginId } },
        body: patch,
      }),
    );
  }

  /** Orders the installed plugins, first to decide first. */
  async orderPlugins(pluginIds: readonly string[]): Promise<void> {
    const result = await this.#client.api.PUT("/api/v1/admin/plugin-order", {
      body: { plugins: [...pluginIds] },
    });
    if (result.error !== undefined) {
      throw new ApiProblemError(problemOf(result.error, result.response));
    }
  }
}
