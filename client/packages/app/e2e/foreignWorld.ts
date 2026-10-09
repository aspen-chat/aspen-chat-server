import type { Page, Route } from "@playwright/test";

/** Another deployment the world's user signs in to from home. */
export const foreignDomain = "beta.example";
const origin = `https://${foreignDomain}`;
/** The user's own account there, a foreign user naming their home. */
const meThere = "0290f0a0-0000-7000-8000-000000000001";
const host = "0290f0a0-0000-7000-8000-000000000002";
export const foreignCommunity = "0290f0a0-0000-7000-8000-000000000010";
/** The one message a search there finds. */
export const foreignSearchText = "The lemonade stand on beta opens at noon.";
/** An invite to its community. */
export const foreignInviteCode = "BetaClub7";
const foreignChannel = "0290f0a0-0000-7000-8000-000000000011";
const foreignDm = "0290f0a0-0000-7000-8000-000000000012";
const everyone = "0290f0a0-0000-7000-8000-000000000040";

const person = (id: string, name: string, extra: Record<string, unknown> = {}) => ({
  id,
  name,
  icon: null,
  onlineStatus: "online",
  displayName: null,
  pronouns: null,
  bio: null,
  status: null,
  bot: false,
  botOwner: null,
  botPublic: false,
  homeDomain: null,
  ...extra,
});

const users = [
  person(meThere, "kate", { homeDomain: "home.example" }),
  person(host, "hostess", { displayName: "Hostess" }),
];

const channelRecord = (id: string, name: string, extra: Record<string, unknown> = {}) => ({
  id,
  parentChannel: null,
  starterMessage: null,
  replyCount: 0,
  lastReplyAt: null,
  recipients: [],
  parentCategory: null,
  community: foreignCommunity,
  name,
  sortIndex: 0,
  ty: "text",
  ...extra,
});

const later = (minutes: number) => new Date(Date.now() + minutes * 60_000).toISOString();
// Newer than anything in the world's own DM, so the merged list shows it first.
const dmMessageId = "0290f0a0-0000-7000-9fff-000000000001";

function json(route: Route, body: unknown, status = 200) {
  return route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body) });
}

/**
 * Stubs `beta.example`: the home lists it among the user's deployments and signs assertions for
 * it, and it signs the user in and holds one community, where they belong, and one DM from its
 * hostess. `signedIn` says whether the home lists it already; otherwise the user adds it.
 */
export async function stubForeignDeployment(page: Page, { listed }: { listed: boolean }) {
  let visited = listed;
  await page.route(/\/api\/v1\/users\/@me\/foreign-deployments$/, (route) =>
    json(
      route,
      visited ? [{ domain: foreignDomain, firstUsedAt: later(-60), lastUsedAt: later(-1) }] : [],
    ),
  );
  await page.route(/\/api\/v1\/auth\/assertions$/, (route) => {
    visited = true;
    return json(route, { assertion: "signed", audience: foreignDomain, expiresAt: later(2) });
  });
  const methods = (federationDomain: string) => ({
    passkeys: null,
    twoFactorRequired: false,
    registrationInviteRequired: false,
    federationDomain,
    protocol: { version: 1, minimum: 1, capabilities: [] },
    software: { name: "aspen", version: "0.1.0" },
  });
  await page.route(/\/api\/v1\/auth\/methods$/, (route) => json(route, methods("home.example")));
  await page.route(`${origin}/api/v1/**`, (route) => {
    const url = new URL(route.request().url());
    const path = decodeURIComponent(url.pathname.replace("/api/v1", ""));
    const method = route.request().method();
    if (method === "POST" && path === "/auth/federated-sign-in") {
      return json(route, {
        userId: meThere,
        refreshToken: "r2",
        sessionToken: "s2",
        sessionTokenExpires: later(60),
        twoFactorEnrollmentRequired: false,
      });
    }
    if (path === "/auth/methods") {
      return json(route, methods(foreignDomain));
    }
    if (path === "/users/@me") {
      return json(route, users[0]);
    }
    if (path === "/users/@me/communities") {
      return json(route, {
        data: [{ id: foreignCommunity, name: "Beta club", icon: null, owner: host }],
        included: {
          channels: [channelRecord(foreignChannel, "lobby")],
          roles: [
            {
              id: everyone,
              community: foreignCommunity,
              name: "everyone",
              position: 0,
              permissions: ["viewChannel", "sendMessages", "createInvites"],
              everyone: true,
            },
          ],
          channelOverrides: [],
          categoryOverrides: [],
          categories: [],
          users,
          userCommunities: [
            { community: foreignCommunity, user: meThere, sortIndex: 0, roles: [] },
            { community: foreignCommunity, user: host, sortIndex: 1, roles: [] },
          ],
          voiceSessions: [],
          voiceParticipants: [],
          readStates: [],
        },
      });
    }
    if (method === "POST" && path === "/users/@me/dms") {
      return json(
        route,
        channelRecord(foreignDm, "", { community: null, ty: "dm", recipients: [meThere, host] }),
      );
    }
    if (path === "/users/@me/dms") {
      return json(route, {
        data: [
          channelRecord(foreignDm, "", {
            community: null,
            ty: "dm",
            recipients: [meThere, host],
          }),
        ],
        included: {
          users,
          readStates: [
            { channel: foreignDm, lastRead: null, lastMessage: dmMessageId, mentions: 0 },
          ],
        },
      });
    }
    if (path === `/invites/${foreignInviteCode}`) {
      return json(route, {
        data: {
          code: foreignInviteCode,
          community: foreignCommunity,
          createdAt: later(-60),
          createdBy: host,
          expiresAt: null,
        },
        included: {
          communities: [{ id: foreignCommunity, name: "Beta club", icon: null, owner: host }],
        },
      });
    }
    if (path === "/messages") {
      // Found wherever it is searched for: newer than anything the home holds.
      return json(route, {
        data: [
          {
            id: "0290f0a0-0000-7000-9fff-000000000002",
            channelId: foreignChannel,
            author: host,
            timestamp: later(-1),
            editedAt: null,
            linkPreviews: [],
            linkedMessages: [],
            alteredBy: [],
            kind: "standard",
            poll: null,
            thread: null,
            echoOf: null,
            content: foreignSearchText,
            attachments: [],
            mentions: { users: [], roles: [], everyone: false },
          },
        ],
        included: { users, channels: [channelRecord(foreignChannel, "lobby")], reactions: [] },
      });
    }
    if (path === "/users/@me/blocks") {
      return json(route, { data: [], included: { users: [] } });
    }
    if (path === "/users/@me/admin") {
      return json(route, { permissions: [], roles: [], inclusions: [] });
    }
    if (path.endsWith("/messages")) {
      return json(route, { data: [], included: { users } });
    }
    if (path === "/users/statuses") {
      return json(route, []);
    }
    // The user's chosen presence, set on every deployment alike, starts unset here too.
    if (path === "/users/@me/presence-override") {
      if (method === "DELETE") {
        return route.fulfill({ status: 204 });
      }
      const asked =
        method === "PUT"
          ? { ...(route.request().postDataJSON() as object), until: null }
          : { presenceOverride: null, until: null };
      return json(route, asked, method === "PUT" ? 201 : 200);
    }
    if (method === "POST" && path === "/auth/logout") {
      return route.fulfill({ status: 204 });
    }
    return json(
      route,
      { code: "notFound", title: `Not stubbed: ${method} ${path}`, status: 404 },
      404,
    );
  });
  await page.routeWebSocket(new RegExp(`^wss://${foreignDomain}/api/v1/events(\\?.*)?$`), (ws) => {
    ws.onMessage((frame) => {
      const parsed = JSON.parse(String(frame)) as { type?: string };
      if (parsed.type === "identify") {
        ws.send(JSON.stringify({ type: "ready", userId: meThere, resumed: false }));
      }
    });
  });
}
