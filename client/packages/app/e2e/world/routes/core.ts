import { signedIn } from "../../stubs";
import {
  archive,
  channels,
  community,
  customEmojiIcon,
  customEmojiId,
  customEmojiName,
  dm,
  dmRecord,
  farMember,
  freshId,
  helper,
  iconRecord,
  inviteCode,
  me,
  message,
  messageId,
  minutesAgo,
  organiserRole,
  planning,
  reactedId,
  roles,
  user,
  users,
  bob,
  communityReadStates,
} from "../fixtures";
import { type Asked, type WorldRoute, reply } from "../reply";

/** The routes of signing in, the community and its channels, messages, emoji, bots, bans, blocks, plugins, muting, notifications, and invites. */
export function coreRoutes({
  route,
  request,
  url,
  path,
  publish,
  blocks,
  bans,
}: Asked): WorldRoute[] {
  return [
    ["GET", /^\/auth\/methods$/, () => ({ passkeys: null, twoFactorRequired: false })],
    ["POST", /^\/auth\/login$/, () => JSON.parse(signedIn()) as unknown],
    [
      "POST",
      /^\/auth\/token-refresh$/,
      () => ({ sessionToken: "s", sessionTokenExpires: minutesAgo(-60) }),
    ],
    ["GET", /^\/users\/@me$/, () => users[0]],
    [
      "GET",
      /^\/users\/@me\/communities$/,
      () => ({
        // Bob owns the community; the signed-in user organises it, managing everything but
        // other people's messages.
        data: [{ id: community, name: "Family", icon: null, owner: bob }],
        included: {
          channels,
          roles,
          channelOverrides: [],
          categoryOverrides: [],
          categories: [
            { id: planning, community, name: "Planning", sortIndex: 0 },
            { id: archive, community, name: "Archive", sortIndex: 1 },
          ],
          users,
          userCommunities: [
            { community, user: me, sortIndex: 0, roles: [organiserRole] },
            { community, user: bob, sortIndex: 1, roles: [] },
            { community, user: helper, sortIndex: null, roles: [] },
          ],
          voiceSessions: [],
          voiceParticipants: [],
          readStates: communityReadStates,
          customEmoji: [
            {
              id: customEmojiId,
              community,
              name: customEmojiName,
              icon: customEmojiIcon,
              createdBy: bob,
            },
          ],
        },
      }),
    ],
    // Icons: the emoji's picture, and the two-phase upload of a new one, whose bytes go to a
    // path under the API the world answers too.
    ["GET", /^\/icons\/[^/]+$/, () => iconRecord(path.split("/")[2] ?? "")],
    [
      "POST",
      /^\/icons$/,
      () =>
        reply(
          {
            id: freshId(),
            uploadUrl: `${url.origin}/api/v1/uploads/icon`,
            expiresAt: new Date(Date.now() + 600_000).toISOString(),
          },
          201,
        ),
    ],
    ["PUT", /^\/uploads\/icon$/, () => reply({}, 200)],
    ["POST", /^\/icons\/[^/]+\/confirm$/, () => iconRecord(path.split("/")[2] ?? "")],
    // The community's emoji: adding, renaming, and removing answer as the server does, and
    // tell everyone by event.
    [
      "POST",
      new RegExp(`^/communities/${community}/emoji$`),
      () => {
        const request = route.request().postDataJSON() as { name: string; icon: string };
        const record = {
          id: freshId(),
          community,
          name: request.name,
          icon: request.icon,
          createdBy: me,
        };
        publish({ serverEvent: "customEmoji", type: "create", ...record });
        return reply(record, 201);
      },
    ],
    [
      "PATCH",
      /^\/emoji\/[^/]+$/,
      () => {
        const id = path.split("/")[2] ?? "";
        const request = route.request().postDataJSON() as { name?: string };
        publish({ serverEvent: "customEmoji", type: "update", id, name: request.name });
        return reply(
          {
            id,
            community,
            name: request.name ?? customEmojiName,
            icon: customEmojiIcon,
            createdBy: bob,
          },
          200,
        );
      },
    ],
    [
      "DELETE",
      /^\/emoji\/[^/]+$/,
      () => {
        publish({ serverEvent: "customEmoji", type: "delete", id: path.split("/")[2] ?? "" });
        return reply(null, 204);
      },
    ],
    [
      "GET",
      /^\/users\/@me\/dms$/,
      () => ({
        data: [dmRecord],
        included: {
          users,
          readStates: [
            { channel: dm, lastRead: messageId(1), lastMessage: messageId(220), mentions: 0 },
          ],
        },
      }),
    ],
    ["PUT", /^\/channels\/[^/]+\/read-states\/@me$/, () => reply(null, 204)],
    ["GET", /^\/channels\/[^/]+\/pins$/, () => []],
    // A member search: everyone in the world whose name holds what was typed, Dana included,
    // though she is not in the member sample.
    [
      "GET",
      new RegExp(`^/communities/${community}/members$`),
      () => {
        const name = (url.searchParams.get("filter[name]") ?? "").toLowerCase();
        const found = [...users, farMember].filter((u) =>
          [u.name, u.displayName ?? ""].some((n) => n.toLowerCase().includes(name)),
        );
        return {
          data: found,
          included: {
            userCommunities: found.map((u) => ({
              community,
              user: u.id,
              sortIndex: 0,
              roles: u.id === me ? [organiserRole] : [],
            })),
          },
        };
      },
    ],
    // Everyone who reacted with an emoji, in one page.
    [
      "GET",
      new RegExp(`^/messages/${reactedId}/reactions/[^/]+$`),
      () => (path.endsWith("👍") ? [users[0], users[1]] : [users[1]]),
    ],
    // Reacting answers with the reaction, which the app applies as its event.
    [
      "PUT",
      /^\/messages\/[^/]+\/reactions\/[^/]+\/@me$/,
      () => {
        const [, , messageId = "", , emoji = ""] = path.split("/");
        return reply({ emoji: decodeURIComponent(emoji), messageId, userId: me }, 201);
      },
    ],
    // Folding a category answers as the server does, and tells the caller's devices by event.
    [
      "PUT",
      /^\/categories\/[^/]+\/collapses\/@me$/,
      () => {
        const category = path.split("/")[2] ?? "";
        publish({ serverEvent: "categoryCollapseChanged", category, collapsed: true });
        return reply({ category }, 201);
      },
    ],
    [
      "DELETE",
      /^\/categories\/[^/]+\/collapses\/@me$/,
      () => {
        const category = path.split("/")[2] ?? "";
        publish({ serverEvent: "categoryCollapseChanged", category, collapsed: false });
        return reply(null, 204);
      },
    ],
    // Posting answers with the message as the server would record it, tagging the people of
    // the world it names.
    [
      "POST",
      /^\/channels\/[^/]+\/messages$/,
      () => {
        const { content } = request.postDataJSON() as { content: string };
        const tagged = users.filter((u) => content.includes(`<@${u.id}>`)).map((u) => u.id);
        return reply(
          message(990, me, content, 0, {
            channelId: path.split("/")[2],
            mentions: { users: tagged, roles: [], everyone: false },
          }),
          201,
        );
      },
    ],
    // Kate's bots: Helper, and whatever she makes, each answered as the server does. Nothing is
    // kept between requests; the app takes its records from the answers.
    ["GET", /^\/users\/@me\/bots$/, () => users.filter((u) => u.botOwner === me)],
    [
      "POST",
      /^\/users\/@me\/bots$/,
      () => {
        const { name, displayName } = request.postDataJSON() as {
          name: string;
          displayName: string | null;
        };
        const bot = {
          ...user(`0190f0a0-0000-7000-8000-${String(Date.now()).slice(-12)}`, name, displayName),
          bot: true,
          botOwner: me,
        };
        return reply({ bot, token: "aspenbot_example" }, 201);
      },
    ],
    ["POST", /^\/bots\/[^/]+\/token$/, () => ({ token: "aspenbot_another" })],
    [
      "PATCH",
      /^\/bots\/[^/]+$/,
      () => {
        const bot = users.find((u) => u.id === path.split("/")[2]);
        const { public: botPublic } = request.postDataJSON() as { public: boolean };
        return bot === undefined ? undefined : { ...bot, botPublic };
      },
    ],
    // Bans: the standing list, banning (which ends the membership and tells everyone), and
    // lifting.
    ["GET", new RegExp(`^/communities/${community}/bans$`), () => Array.from(bans.values())],
    [
      "PUT",
      new RegExp(`^/communities/${community}/bans/[^/]+$`),
      () => {
        const user = path.split("/").pop() ?? "";
        const request = route.request().postDataJSON() as {
          reason?: string;
          durationSeconds?: number;
          deleteMessagesSeconds?: number;
        };
        const ban = {
          community,
          user,
          reason: request.reason ?? null,
          until:
            request.durationSeconds === undefined
              ? null
              : new Date(Date.now() + request.durationSeconds * 1000).toISOString(),
          bannedBy: me,
          bannedAt: new Date().toISOString(),
        };
        const replaced = bans.has(user);
        bans.set(user, ban);
        publish({ serverEvent: "userCommunity", type: "delete", community, user });
        publish({ serverEvent: "communityBan", type: "create", ...ban });
        return reply({ ban, deletedMessages: 0 }, replaced ? 200 : 201);
      },
    ],
    [
      "DELETE",
      new RegExp(`^/communities/${community}/bans/[^/]+$`),
      () => {
        const user = path.split("/").pop() ?? "";
        bans.delete(user);
        publish({ serverEvent: "communityBan", type: "delete", community, user });
        return reply(null, 204);
      },
    ],
    [
      "PUT",
      new RegExp(`^/communities/${community}/members/[^/@][^/]*$`),
      () => reply({ community, user: path.split("/").pop(), sortIndex: null, roles: [] }, 201),
    ],
    // The deployment runs no plugins, and they say nothing about anyone; `plugins.spec.ts`
    // answers these itself.
    ["GET", /^\/plugins$/, () => []],
    ["GET", /^\/users\/[^/]+\/annotations$/, () => []],
    // Blocking answers as the server does, and tells the caller's devices by event.
    [
      "GET",
      /^\/users\/@me\/blocks$/,
      () => ({
        data: Array.from(blocks, (user) => ({ user, createdAt: minutesAgo(1) })),
        included: { users: users.filter((u) => blocks.has(u.id)) },
      }),
    ],
    [
      "PUT",
      /^\/users\/@me\/blocks\/[^/]+$/,
      () => {
        const user = path.split("/").pop() ?? "";
        blocks.add(user);
        publish({ serverEvent: "userBlockChanged", user, blocked: true });
        return reply({ user, createdAt: minutesAgo(0) }, 201);
      },
    ],
    [
      "DELETE",
      /^\/users\/@me\/blocks\/[^/]+$/,
      () => {
        const user = path.split("/").pop() ?? "";
        blocks.delete(user);
        publish({ serverEvent: "userBlockChanged", user, blocked: false });
        return reply(null, 204);
      },
    ],
    // Muting answers as the server does, and tells the caller's devices by event.
    [
      "PUT",
      /^\/channels\/[^/]+\/mutes\/@me$/,
      () => {
        const channel = path.split("/")[2] ?? "";
        const { durationSeconds } = request.postDataJSON() as { durationSeconds: number | null };
        const until =
          durationSeconds === null
            ? null
            : new Date(Date.now() + durationSeconds * 1000).toISOString();
        publish({ serverEvent: "channelMuteChanged", channel, muted: true, until });
        return reply({ channel, until }, 201);
      },
    ],
    [
      "PUT",
      /^\/(channels|communities)\/[^/]+\/notification-settings\/@me$/,
      () => {
        const [, kind, id] = path.split("/");
        const { level } = request.postDataJSON() as { level: string };
        const setting = {
          community: kind === "communities" ? (id ?? null) : null,
          channel: kind === "channels" ? (id ?? null) : null,
          level,
        };
        publish({ serverEvent: "notificationSettingChanged", ...setting });
        return reply(setting, 201);
      },
    ],
    [
      "DELETE",
      /^\/(channels|communities)\/[^/]+\/notification-settings\/@me$/,
      () => {
        const [, kind, id] = path.split("/");
        publish({
          serverEvent: "notificationSettingChanged",
          community: kind === "communities" ? (id ?? null) : null,
          channel: kind === "channels" ? (id ?? null) : null,
          level: null,
        });
        return reply(null, 204);
      },
    ],
    [
      "DELETE",
      /^\/channels\/[^/]+\/mutes\/@me$/,
      () => {
        publish({
          serverEvent: "channelMuteChanged",
          channel: path.split("/")[2] ?? "",
          muted: false,
        });
        return reply(null, 204);
      },
    ],
    [
      "GET",
      new RegExp(`^/invites/${inviteCode}$`),
      () => ({
        data: {
          code: inviteCode,
          community,
          createdAt: minutesAgo(60),
          createdBy: me,
          expiresAt: null,
        },
        included: { communities: [{ id: community, name: "Family", icon: null, owner: bob }] },
      }),
    ],
    [
      "GET",
      new RegExp(`^/communities/${community}/invites$`),
      () => [
        {
          code: inviteCode,
          community,
          createdAt: minutesAgo(60),
          createdBy: me,
          expiresAt: null,
        },
      ],
    ],
  ];
}
