import type { Page, Route } from "@playwright/test";
import { signedIn, uuid } from "./stubs";

/**
 * A small signed-in world for specs that need more than an empty account: one community with a
 * text channel, a voice channel, and a category; a conversation in the text channel with a
 * thread and an echoed reply, then a poll of Bob's that takes write-ins; and a one-to-one DM.
 * The caller has read #general up to Bob's "Sounds good" (`lastReadText`) and has not read the
 * DM. #roadmap shows as unread without any history, for checking the channel list alone;
 * #ideas, beside it in Planning, is read. The Archive category is empty. Every API read the app makes about it is
 * answered from here, and anything else is refused with a Problem naming the request, so a
 * spec fails loudly rather than waiting on a request nobody answers.
 */

export const me = uuid;
export const bob = "0190f0a0-0000-7000-8000-000000000002";
export const community = "0190f0a0-0000-7000-8000-000000000010";
/** Kate's bot, a member of the community. */
export const helper = "0190f0a0-0000-7000-8000-000000000003";
const everyoneRole = "0190f0a0-0000-7000-8000-000000000040";
const deploymentAdministrator = "0190f0a0-0000-7000-8000-000000000042";
const organiserRole = "0190f0a0-0000-7000-8000-000000000041";
const channelPermissions = [
  "viewChannel",
  "sendMessages",
  "attachFiles",
  "addReactions",
  "startThreads",
  "sendInThreads",
  "createPolls",
  "joinVoice",
  "speak",
  "shareScreen",
  "mentionMembers",
];
const roles = [
  {
    id: everyoneRole,
    community,
    name: "everyone",
    position: 0,
    permissions: [...channelPermissions, "createInvites"],
    everyone: true,
  },
  {
    id: organiserRole,
    community,
    name: "Organiser",
    position: 1,
    permissions: [
      "manageCommunity",
      "manageChannels",
      "manageCategories",
      "createInvites",
      "manageInvites",
      "manageRoles",
      "assignRoles",
      "removeMembers",
      "pinMessages",
      "manageCalls",
      "addBots",
      ...channelPermissions,
    ],
    everyone: false,
  },
];
export const general = "0190f0a0-0000-7000-8000-000000000011";
export const lounge = "0190f0a0-0000-7000-8000-000000000012";
export const planning = "0190f0a0-0000-7000-8000-000000000013";
export const roadmap = "0190f0a0-0000-7000-8000-000000000014";
export const thread = "0190f0a0-0000-7000-8000-000000000015";
export const dm = "0190f0a0-0000-7000-8000-000000000016";
export const lunchPoll = "0190f0a0-0000-7000-8000-000000000017";
export const ideas = "0190f0a0-0000-7000-8000-000000000018";
/** A category with no channels. */
export const archive = "0190f0a0-0000-7000-8000-000000000019";

const minutesAgo = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();

const user = (id: string, name: string, displayName: string | null) => ({
  id,
  onlineStatus: "online",
  name,
  icon: null,
  displayName,
  pronouns: null,
  bio: null,
  status: null,
  bot: false,
  botOwner: null as string | null,
  botPublic: false,
});

/** A member of the community beyond its member sample, whom only a search finds. */
const farMember = user("0190f0a0-0000-7000-8000-000000000004", "dana", "Dana From Far Away");

const users = [
  user(me, "kate", "Kate"),
  user(bob, "bob", "Bob With A Rather Long Display Name"),
  { ...user(helper, "helper", "Helper"), bot: true, botOwner: me },
];

const channel = (
  id: string,
  name: string,
  ty: string,
  extra: Record<string, unknown> = {},
): Record<string, unknown> => ({
  id,
  parentChannel: null,
  starterMessage: null,
  replyCount: 0,
  lastReplyAt: null,
  recipients: [],
  parentCategory: null,
  community,
  name,
  sortIndex: 0,
  ty,
  ...extra,
});

const channels = [
  channel(general, "general", "text"),
  channel(lounge, "Lounge", "voice", { sortIndex: 1 }),
  channel(roadmap, "roadmap", "text", { parentCategory: planning }),
  channel(ideas, "ideas", "text", { parentCategory: planning, sortIndex: 1 }),
];

// Ids grow with time, as UUIDv7 ones do: a window is ordered by them.
const messageId = (n: number) => `0190f0a0-0000-7000-8001-${String(n).padStart(12, "0")}`;

const message = (
  n: number,
  author: string,
  content: string,
  minutes: number,
  extra: Record<string, unknown> = {},
): Record<string, unknown> => ({
  id: messageId(n),
  channelId: general,
  author,
  timestamp: minutesAgo(minutes),
  editedAt: null,
  linkPreviews: [],
  kind: "standard",
  poll: null,
  thread: null,
  echoOf: null,
  content,
  attachments: [],
  mentions: { users: [], roles: [], everyone: false },
  ...extra,
});

/** The thread's starter, in #general. */
export const starterText = "Who is bringing snacks on Saturday?";
/** A message of the caller's own, which offers editing and deleting. */
export const ownText = "I can bring the lemonade and some chairs.";

const threadRecord = channel(thread, "", "thread", {
  parentChannel: general,
  starterMessage: messageId(203),
  replyCount: 2,
  lastReplyAt: minutesAgo(20),
});

const threadReplies = [
  message(210, bob, "Crisps and a fruit platter from me.", 25, { channelId: thread }),
  message(211, me, "Perfect, thank you!", 20, { channelId: thread }),
];

/** The question of Bob's poll, the newest message in #general. */
export const pollQuestion = "Where should we have lunch?";

// Newest first, as the server answers.
const generalMessages = [
  message(207, bob, "", 3, { kind: "poll", poll: lunchPoll }),
  message(206, me, ownText, 5),
  message(205, bob, "", 20, { kind: "threadEcho", echoOf: messageId(211) }),
  message(204, bob, "Sounds good. See everyone at ten, and bring a jumper in case it rains.", 40),
  message(203, me, starterText, 60, { thread }),
  message(202, bob, "Morning all!", 90),
  message(201, me, "Welcome to the family server.", 120),
  // Older history, enough for several pages, so reading back through it can be exercised.
  ...Array.from({ length: 120 }, (_, i) =>
    message(120 - i, i % 2 === 0 ? me : bob, `Older message ${String(120 - i)}`, 180 + i * 10),
  ),
];

/** The last message in #general the caller has read; everything after it is new. */
export const lastReadText =
  "Sounds good. See everyone at ten, and bring a jumper in case it rains.";

/** #roadmap holds unread messages, two of which tag the caller. */
const communityReadStates = [
  { channel: general, lastRead: messageId(204), lastMessage: messageId(207), mentions: 0 },
  { channel: lounge, lastRead: messageId(1), lastMessage: null, mentions: 0 },
  { channel: roadmap, lastRead: messageId(1), lastMessage: messageId(230), mentions: 2 },
  { channel: ideas, lastRead: messageId(1), lastMessage: null, mentions: 0 },
];

/** Bob's "Sounds good" message, which has a crowd of reactions. */
export const reactedText = lastReadText;
const reactedId = messageId(204);

/** Emoji with one reaction each, in the order first used, after the popular two. */
export const singleReactions = [
  "😀",
  "😂",
  "🥲",
  "😍",
  "🤔",
  "😎",
  "🙃",
  "😴",
  "🤯",
  "🥳",
  "😇",
  "🤖",
  "👻",
  "🐱",
  "🐶",
  "🍕",
  "🌮",
  "☕",
  "🚀",
  "🌈",
];

/**
 * The reactions to Bob's message, in brief, emoji in the order first used: one each for the
 * singles, six 🎉, and seven 👍 (Kate and Bob first), used last but the most popular.
 */
const reactionSummaries = [
  ...singleReactions.slice(0, 1).map((emoji) => ({
    messageId: reactedId,
    emoji,
    count: 1,
    me: false,
    users: [bob],
  })),
  { messageId: reactedId, emoji: "🎉", count: 6, me: false, users: [bob] },
  ...singleReactions.slice(1).map((emoji) => ({
    messageId: reactedId,
    emoji,
    count: 1,
    me: false,
    users: [bob],
  })),
  { messageId: reactedId, emoji: "👍", count: 7, me: true, users: [me, bob] },
];

/** The community's one invite. */
export const inviteCode = "FamilyInvite42";

/** How long a page of older history takes to arrive, long enough to see it loading. */
export const HISTORY_DELAY_MS = 400;

/** A page of #general, newest first, as `GET /channels/{general}/messages` answers it. */
function generalPage(url: URL) {
  const limit = Number(url.searchParams.get("limit") ?? "50");
  const before = url.searchParams.get("before");
  const start = before === null ? 0 : generalMessages.findIndex((m) => m.id === before) + 1;
  return generalMessages.slice(start, start + limit);
}

const dmRecord = {
  ...channel(dm, "", "dm"),
  community: null,
  recipients: [me, bob],
};

/** Bob's message in the DM, which tags the caller. */
const dmMessages = [
  message(220, bob, `<@${me}> Did you get the photos?`, 30, {
    channelId: dm,
    mentions: { users: [me], roles: [], everyone: false },
  }),
];
/** The one message in the DM, which the caller has not read. */
export const dmMessageId = messageId(220);

/**
 * A message search as the server answers it: messages holding every word of `filter[text]`,
 * ignoring case, in `filter[channel]` and its thread when given, newest first, with their
 * authors and channels.
 */
function searchMessages(url: URL) {
  const words = (url.searchParams.get("filter[text]") ?? "")
    .toLowerCase()
    .split(/\s+/)
    .filter((word) => word !== "");
  const within = url.searchParams.get("filter[channel]");
  const found = [...generalMessages, ...threadReplies, ...dmMessages]
    .filter((m) => m.kind !== "threadEcho" && m.kind !== "poll")
    .filter((m) => words.every((word) => String(m.content).toLowerCase().includes(word)))
    .filter(
      (m) =>
        within === null || m.channelId === within || (within === general && m.channelId === thread),
    )
    .sort((a, b) => String(b.id).localeCompare(String(a.id)))
    .slice(0, 25);
  const posted = new Set(found.map((m) => m.channelId));
  return {
    data: found,
    included: {
      users,
      channels: [...channels, threadRecord, dmRecord].filter((c: Record<string, unknown>) =>
        posted.has(c.id),
      ),
      reactions: [],
    },
  };
}

/** Sends a server event down the page's event stream. */
type Publish = (event: Record<string, unknown>) => void;

/** A registration invite of the world's, as the admin API lists it. */
interface WorldInvite {
  code: string;
  createdBy: string | null;
  createdAt: string;
  expiresAt: string | null;
  maxUses: number;
  uses: number;
  revokedAt: string | null;
  note: string | null;
  usable: boolean;
}

/** The invite already made from the terminal, used once of its two uses. */
export const standingInvite = "Terminal01";

/**
 * The Administration Dashboard's side of the world: the caller administers it; its invites
 * change as a spec makes and revokes them, one set per page.
 */
function administration() {
  const invites: WorldInvite[] = [
    {
      code: standingInvite,
      createdBy: null,
      createdAt: minutesAgo(600),
      expiresAt: null,
      maxUses: 2,
      uses: 1,
      revokedAt: null,
      note: "for the family",
      usable: true,
    },
  ];
  const directory = [
    {
      id: bob,
      name: "bob",
      displayName: "Bob With A Rather Long Display Name",
      icon: null,
      createdAt: minutesAgo(900),
      roles: [] as string[],
      registeredWith: standingInvite,
    },
    {
      id: me,
      name: "kate",
      displayName: "Kate",
      icon: null,
      createdAt: minutesAgo(1200),
      roles: [deploymentAdministrator],
      registeredWith: null,
    },
  ];
  // Twenty more people who joined a day apart, so the list runs to more than one page.
  for (let i = 1; i <= 20; i++) {
    directory.push({
      id: `0190f0a0-0000-7000-8000-0000000001${String(i).padStart(2, "0")}`,
      name: `member${String(i).padStart(2, "0")}`,
      displayName: `Member ${String(i).padStart(2, "0")}`,
      icon: null,
      createdAt: minutesAgo(1440 * i + 1500),
      roles: [] as string[],
      registeredWith: standingInvite,
    });
  }
  const communities = [
    { id: community, name: "Family", icon: null, members: 2, createdAt: minutesAgo(3000) },
    {
      id: "0190f0a0-0000-7000-8000-000000000020",
      name: "Book club",
      icon: null,
      members: 9,
      createdAt: minutesAgo(9000),
    },
  ];
  const named = (url: URL) => (url.searchParams.get("filter[name]") ?? "").toLowerCase();
  /** A page of `rows` as the server gives it: sorted by `sort`, from `offset`, `limit` long. */
  function page<T extends Record<string, unknown>>(
    rows: T[],
    url: URL,
    keyOf: (row: T, field: string) => string | number,
  ): T[] {
    const sort = url.searchParams.get("sort") ?? "-createdAt";
    const field = sort.replace(/^-/, "");
    const sign = sort.startsWith("-") ? -1 : 1;
    const sorted = [...rows].sort((a, b) => {
      const x = keyOf(a, field);
      const y = keyOf(b, field);
      return (x < y ? -1 : x > y ? 1 : 0) * sign;
    });
    const offset = Number(url.searchParams.get("offset") ?? "0");
    const limit = Number(url.searchParams.get("limit") ?? "15");
    return sorted.slice(offset, offset + limit);
  }
  return {
    overview: () => ({
      users: 22,
      newUsersThisWeek: 2,
      communities: 2,
      registrationInviteRequired: true,
    }),
    users: (url: URL) =>
      page(
        directory.filter((u) =>
          [u.name, u.displayName].some((n) => n.toLowerCase().includes(named(url))),
        ),
        url,
        (u, field) => (field === "name" ? u.displayName.toLowerCase() : u.createdAt),
      ),
    communities: (url: URL) =>
      page(
        communities.filter((c) => c.name.toLowerCase().includes(named(url))),
        url,
        (c, field) =>
          field === "name" ? c.name.toLowerCase() : field === "members" ? c.members : c.createdAt,
      ),
    growth: (url: URL) => {
      const monthly = url.searchParams.get("range") === "fiveYears";
      const steps = monthly ? 61 : 92;
      return {
        unit: monthly ? "month" : "day",
        points: Array.from({ length: steps }, (_, i) => {
          const back = steps - 1 - i;
          const at = new Date(Date.now() - back * (monthly ? 30 : 1) * 86_400_000);
          return {
            at: at.toISOString(),
            users: Math.round((monthly ? 4 : 380) + i * (monthly ? 6 : 0.4)),
            communities: Math.round((monthly ? 1 : 30) + i * (monthly ? 0.5 : 0.08)),
          };
        }),
      };
    },
    invites: () => invites,
    federation: federationWorld(),
    create: (body: { maxUses?: number; note?: string }) => {
      const invite: WorldInvite = {
        code: `Made${String(invites.length).padStart(4, "0")}`,
        createdBy: me,
        createdAt: new Date().toISOString(),
        expiresAt: null,
        maxUses: body.maxUses ?? 1,
        uses: 0,
        revokedAt: null,
        note: body.note ?? null,
        usable: true,
      };
      invites.unshift(invite);
      return reply(invite, 201);
    },
    revoke: (code: string) => {
      const invite = invites.find((i) => i.code === code);
      if (invite !== undefined) {
        invite.revokedAt = new Date().toISOString();
        invite.usable = false;
      }
      return reply(null, 204);
    },
    fleet: () => ({
      apiServers: [
        {
          instance: "a",
          host: "api-1",
          version: "0.1.0",
          startedAt: minutesAgo(60 * 26),
          reportedAt: minutesAgo(0),
          eventStreams: 42,
          requestsPerMinute: 318.5,
          serverErrorsPerMinute: 0,
          residentBytes: 214 * 1024 * 1024,
          dbConnections: 8,
          dbConnectionsIdle: 6,
        },
        {
          instance: "b",
          host: "api-2",
          version: "0.1.0",
          startedAt: minutesAgo(90),
          reportedAt: minutesAgo(0),
          eventStreams: 17,
          requestsPerMinute: 120,
          serverErrorsPerMinute: 2.5,
          residentBytes: 180 * 1024 * 1024,
          dbConnections: 4,
          dbConnectionsIdle: 4,
        },
      ],
      voiceServers: [
        {
          id: "0190f0a0-0000-7000-8000-000000000031",
          name: "voice-east",
          url: "https://voice-east.example",
          enabled: true,
          capacity: 500,
          participants: 12,
          lastReportAt: minutesAgo(0),
          reporting: true,
        },
        {
          id: "0190f0a0-0000-7000-8000-000000000032",
          name: "voice-west",
          url: "https://voice-west.example",
          enabled: true,
          capacity: 500,
          participants: 0,
          lastReportAt: minutesAgo(45),
          reporting: false,
        },
        {
          id: "0190f0a0-0000-7000-8000-000000000033",
          name: "voice-spare",
          url: "https://voice-spare.example",
          enabled: false,
          capacity: 100,
          participants: 0,
          lastReportAt: null,
          reporting: false,
        },
      ],
    }),
  };
}

/**
 * Bob's poll, one per page so that what a spec writes in stays in that spec. Bob has written in
 * an answer already; the caller has written in none. Each change is published as the poll's
 * `update` event, as the server does.
 */
function lunch(publish: Publish) {
  const poll = {
    id: lunchPoll,
    channelId: general,
    messageId: messageId(207),
    createdBy: bob,
    createdAt: minutesAgo(3),
    closesAt: minutesAgo(-60),
    closedAt: null,
    question: pollQuestion,
    options: [{ label: "Pizza" }, { label: "Sushi" }],
    multipleChoice: false,
    allowWriteIns: true,
    writeIns: [{ label: "Pancakes", writtenBy: bob }] as ({
      label: string;
      writtenBy: string;
    } | null)[],
    anonymous: false,
    results: [
      { count: 1, voters: [bob] },
      { count: 0, voters: [] },
      { count: 0, voters: [] },
    ] as { count: number; voters: string[] }[],
  };
  const changed = () => {
    publish({
      serverEvent: "poll",
      type: "update",
      id: poll.id,
      writeIns: poll.writeIns,
      results: poll.results,
    });
  };
  return {
    read: () => ({ data: poll, included: { pollVotes: [], ownWriteIns: [] } }),
    writeIn: (label: string) => {
      poll.writeIns.push({ label, writtenBy: me });
      poll.results.push({ count: 1, voters: [me] });
      changed();
      return reply({ option: poll.results.length - 1, poll }, 201);
    },
    remove: (option: number) => {
      poll.writeIns[option - poll.options.length] = null;
      poll.results[option] = { count: 0, voters: [] };
      changed();
      return reply(null, 204);
    },
  };
}

/** Another deployment of the world's, as the admin API lists it. */
interface WorldDeployment {
  domain: string;
  origin: "administrator" | "terminal" | "firstContact";
  addedBy: string | null;
  createdAt: string;
  note: string | null;
  publicKey: string | null;
  publicKeyFingerprint: string | null;
  firstContactAt: string | null;
  lastContactAt: string | null;
  offeredKey: string | null;
  offeredKeyFingerprint: string | null;
  offeredKeyAt: string | null;
  lists: string[];
  protocol: { version: number; minimum: number; capabilities: string[] } | null;
  software: { name: string; version: string } | null;
  compatible: boolean;
  admission: {
    usersEmigration: boolean;
    usersImmigration: boolean;
    botsEmigration: boolean;
    botsImmigration: boolean;
  };
}

/** The deployment the world knows whose key has changed since it was pinned. */
export const rekeyedDeployment = "chat.example.org";
/** The deployment the world knows that answers with its pinned key. */
export const friendlyDeployment = "friends.example.net:8443";

/**
 * The world's federation: this deployment lets its users go only where it allows and takes
 * anyone's, and keeps its bots home. It knows two deployments; the first has offered a new key.
 */
function federationWorld() {
  const admission = (lists: string[]) => ({
    usersEmigration: lists.includes("usersEmigrationAllow"),
    usersImmigration: true,
    botsEmigration: false,
    botsImmigration: false,
  });
  const known = (domain: string, extra: Partial<WorldDeployment>): WorldDeployment => ({
    domain,
    origin: "administrator",
    addedBy: me,
    createdAt: minutesAgo(3000),
    note: null,
    publicKey: "cGlubmVk",
    publicKeyFingerprint: `SHA256:pinned-${domain}`,
    firstContactAt: minutesAgo(3000),
    lastContactAt: minutesAgo(60),
    offeredKey: null,
    offeredKeyFingerprint: null,
    offeredKeyAt: null,
    lists: [],
    admission: admission([]),
    protocol: { version: 1, minimum: 1, capabilities: [] },
    software: { name: "aspen", version: "0.1.0" },
    compatible: true,
    ...extra,
  });
  const deployments: WorldDeployment[] = [
    known(rekeyedDeployment, {
      note: "the neighbours",
      offeredKey: "bmV3",
      offeredKeyFingerprint: "SHA256:offered-key",
      offeredKeyAt: minutesAgo(5),
    }),
    known(friendlyDeployment, { origin: "firstContact", addedBy: null }),
  ];
  const find = (domain: string) => deployments.find((d) => d.domain === domain);
  return {
    overview: () => ({
      domain: "aspen.example.com",
      keyFingerprint: "SHA256:this-deployment",
      keyCreatedAt: minutesAgo(9000),
      users: { emigration: "allowList", immigration: "open" },
      bots: { emigration: "closed", immigration: "closed" },
      usersSharedList: false,
      botsSharedList: false,
      listsInForce: ["usersEmigrationAllow"],
      document: null,
      protocol: { version: 1, minimum: 1, capabilities: [] },
      software: { name: "aspen", version: "0.1.0" },
    }),
    list: (url: URL) => {
      const name = (url.searchParams.get("filter[name]") ?? "").toLowerCase();
      return deployments.filter((d) => d.domain.includes(name));
    },
    add: (body: { domain: string; note?: string }) => {
      const domain = body.domain.trim().toLowerCase();
      if (find(domain) !== undefined) {
        return reply(
          {
            code: "conflict",
            title: "Conflict",
            status: 409,
            detail: "That deployment is already in the directory.",
          },
          409,
        );
      }
      const added = known(domain, {
        note: body.note ?? null,
        publicKey: null,
        publicKeyFingerprint: null,
        firstContactAt: null,
        lastContactAt: null,
      });
      deployments.push(added);
      return reply(added, 201);
    },
    contact: (domain: string) => {
      const deployment = find(domain);
      if (deployment === undefined) {
        return reply({ code: "notFound", title: "Not found", status: 404 }, 404);
      }
      const now = new Date().toISOString();
      const first = deployment.publicKey === null;
      if (first) {
        deployment.publicKey = "Zmlyc3Q";
        deployment.publicKeyFingerprint = "SHA256:first-key";
        deployment.firstContactAt = now;
      }
      deployment.lastContactAt = now;
      const outcome =
        deployment.offeredKey !== null ? "keyChanged" : first ? "pinned" : "confirmed";
      return { outcome, deployment };
    },
    accept: (domain: string, publicKey: string) => {
      const deployment = find(domain);
      if (deployment?.offeredKey !== publicKey) {
        return reply({ code: "conflict", title: "Conflict", status: 409 }, 409);
      }
      deployment.publicKey = deployment.offeredKey;
      deployment.publicKeyFingerprint = deployment.offeredKeyFingerprint;
      deployment.offeredKey = null;
      deployment.offeredKeyFingerprint = null;
      deployment.offeredKeyAt = null;
      return deployment;
    },
    setListed: (domain: string, list: string, on: boolean) => {
      const deployment = find(domain);
      if (deployment === undefined) {
        return reply({ code: "notFound", title: "Not found", status: 404 }, 404);
      }
      const had = deployment.lists.includes(list);
      deployment.lists = on
        ? [...new Set([...deployment.lists, list])]
        : deployment.lists.filter((l) => l !== list);
      deployment.admission = admission(deployment.lists);
      return on ? reply(deployment, had ? 200 : 201) : reply(null, 204);
    },
    forget: (domain: string) => {
      const at = deployments.findIndex((d) => d.domain === domain);
      if (at >= 0) {
        deployments.splice(at, 1);
      }
      return reply(null, 204);
    },
  };
}

/** A response other than `200` with a JSON body. */
class Reply {
  constructor(
    readonly body: unknown,
    readonly status: number,
  ) {}
}

const reply = (body: unknown, status: number) => new Reply(body, status);

function json(route: Route, body: unknown, status = 200) {
  return route.fulfill({ status, contentType: "application/json", body: JSON.stringify(body) });
}

/** Answers every API request the app makes about the world above. */
async function answer(
  route: Route,
  poll: ReturnType<typeof lunch>,
  publish: Publish,
  admin: ReturnType<typeof administration>,
  blocks: Set<string>,
) {
  const request = route.request();
  const url = new URL(request.url());
  const path = decodeURIComponent(url.pathname.replace(/^.*\/api\/v1/, ""));
  const method = request.method();
  const routes: [string, RegExp, () => unknown][] = [
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
        },
      }),
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
    [
      "PUT",
      new RegExp(`^/communities/${community}/members/[^/@][^/]*$`),
      () => reply({ community, user: path.split("/").pop(), sortIndex: null, roles: [] }, 201),
    ],
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
    ["GET", /^\/admin\/federation$/, admin.federation.overview],
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
    ["GET", /^\/messages$/, () => searchMessages(url)],
    ["GET", /^\/users\/@me\/preferences$/, () => ({ values: {}, updatedAt: minutesAgo(600) })],
    [
      "PATCH",
      /^\/users\/@me\/preferences$/,
      () => ({ values: request.postDataJSON() as unknown, updatedAt: minutesAgo(0) }),
    ],
    ["GET", /^\/users\/statuses$/, () => users.map((u) => ({ id: u.id, onlineStatus: "online" }))],
    [
      "GET",
      new RegExp(`^/channels/${general}/messages$`),
      () => ({
        data: generalPage(url),
        included: {
          users,
          channels: [threadRecord],
          messages: [threadReplies[1]],
          reactions: generalPage(url).some((m) => m.id === reactedId) ? reactionSummaries : [],
        },
      }),
    ],
    [
      "GET",
      new RegExp(`^/channels/${thread}/messages$`),
      () => ({ data: [...threadReplies].reverse(), included: { users } }),
    ],
    [
      "GET",
      new RegExp(`^/channels/${dm}/messages$`),
      () => ({ data: dmMessages, included: { users } }),
    ],
    ["GET", /^\/channels\/[^/]+\/messages$/, () => ({ data: [], included: { users } })],
    ["GET", new RegExp(`^/channels/${thread}$`), () => threadRecord],
    ["GET", new RegExp(`^/polls/${lunchPoll}$`), poll.read],
    [
      "POST",
      new RegExp(`^/polls/${lunchPoll}/write-ins$`),
      () => poll.writeIn((request.postDataJSON() as { label: string }).label),
    ],
    [
      "DELETE",
      new RegExp(`^/polls/${lunchPoll}/write-ins/\\d+$`),
      () => poll.remove(Number(path.split("/").pop())),
    ],
    [
      "GET",
      new RegExp(`^/messages/${messageId(203)}$`),
      () => ({
        data: generalMessages.find((m) => m.id === messageId(203)),
        included: { users, channels: [threadRecord], attachments: [], polls: [], pollVotes: [] },
      }),
    ],
  ];
  if (url.searchParams.has("before")) {
    await new Promise((resolve) => setTimeout(resolve, HISTORY_DELAY_MS));
  }
  const match = routes.find(([m, pattern]) => m === method && pattern.test(path));
  if (match === undefined) {
    return route.fulfill({
      status: 404,
      contentType: "application/problem+json",
      body: JSON.stringify({
        code: "notFound",
        title: `Not stubbed: ${method} ${path}`,
        status: 404,
      }),
    });
  }
  const result = match[2]();
  if (result instanceof Reply) {
    return result.body === null
      ? route.fulfill({ status: result.status })
      : json(route, result.body, result.status);
  }
  return json(route, result);
}

/**
 * Answers the event stream's `identify` with `ready`, then sends only what the returned
 * `publish` is given.
 */
async function events(page: Page): Promise<Publish> {
  let send: (frame: string) => void = () => undefined;
  let sequence = 0;
  await page.routeWebSocket(/\/api\/v1\/events(\?.*)?$/, (ws) => {
    send = (frame) => {
      ws.send(frame);
    };
    ws.onMessage((frame) => {
      const parsed: unknown = JSON.parse(String(frame));
      if (
        typeof parsed === "object" &&
        parsed !== null &&
        "type" in parsed &&
        parsed.type === "identify"
      ) {
        ws.send(JSON.stringify({ type: "ready", userId: me, resumed: false }));
      }
    });
  });
  return (event) => {
    sequence += 1;
    send(JSON.stringify({ type: "event", sequence, event }));
  };
}

/** Stubs the world and signs in through the form, as a user would. */
export async function signInToWorld(
  page: Page,
  /** Routes that answer before the world's own, registered after it so they win. */
  before?: (page: Page) => Promise<void>,
): Promise<void> {
  const publish = await events(page);
  const poll = lunch(publish);
  const admin = administration();
  const blocks = new Set<string>();
  await page.route(/\/api\/v1\//, (route) => answer(route, poll, publish, admin, blocks));
  await before?.(page);
  await page.goto("/");
  await page.getByLabel("Username").fill("kate");
  await page.getByLabel("Password").fill("hunter22");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
}
