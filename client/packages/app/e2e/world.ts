import type { Page, Route } from "@playwright/test";
import { signedIn, uuid } from "./stubs";

/**
 * A small signed-in world for specs that need more than an empty account: one community with a
 * text channel, a voice channel, and a category; a conversation in the text channel with a
 * thread and an echoed reply, then a poll of Bob's that takes write-ins; and a one-to-one DM.
 * The caller has read #general up to Bob's "Sounds good" (`lastReadText`) and has not read the
 * DM. #roadmap shows as unread without any history, for checking the channel list alone. Every API read the app makes about it is
 * answered from here, and anything else is refused with a Problem naming the request, so a
 * spec fails loudly rather than waiting on a request nobody answers.
 */

export const me = uuid;
export const bob = "0190f0a0-0000-7000-8000-000000000002";
export const community = "0190f0a0-0000-7000-8000-000000000010";
export const general = "0190f0a0-0000-7000-8000-000000000011";
export const lounge = "0190f0a0-0000-7000-8000-000000000012";
export const planning = "0190f0a0-0000-7000-8000-000000000013";
export const roadmap = "0190f0a0-0000-7000-8000-000000000014";
export const thread = "0190f0a0-0000-7000-8000-000000000015";
export const dm = "0190f0a0-0000-7000-8000-000000000016";
export const lunchPoll = "0190f0a0-0000-7000-8000-000000000017";

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
});

const users = [user(me, "kate", "Kate"), user(bob, "bob", "Bob With A Rather Long Display Name")];

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

const communityReadStates = [
  { channel: general, lastRead: messageId(204), lastMessage: messageId(207) },
  { channel: lounge, lastRead: messageId(1), lastMessage: null },
  { channel: roadmap, lastRead: messageId(1), lastMessage: messageId(230) },
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

const dmMessages = [message(220, bob, "Did you get the photos?", 30, { channelId: dm })];
/** The one message in the DM, which the caller has not read. */
export const dmMessageId = messageId(220);

/** Sends a server event down the page's event stream. */
type Publish = (event: Record<string, unknown>) => void;

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
async function answer(route: Route, poll: ReturnType<typeof lunch>) {
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
        data: [{ id: community, name: "Family", icon: null }],
        included: {
          channels,
          categories: [{ id: planning, community, name: "Planning", sortIndex: 0 }],
          users,
          userCommunities: [
            { community, user: me, sortIndex: 0 },
            { community, user: bob, sortIndex: 1 },
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
          readStates: [{ channel: dm, lastRead: messageId(1), lastMessage: messageId(220) }],
        },
      }),
    ],
    ["PUT", /^\/channels\/[^/]+\/read-states\/@me$/, () => reply(null, 204)],
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
    ["GET", /^\/users\/@me\/preferences$/, () => ({ values: {}, updatedAt: minutesAgo(600) })],
    ["GET", /^\/users\/statuses$/, () => users.map((u) => ({ id: u.id, onlineStatus: "online" }))],
    [
      "GET",
      new RegExp(`^/channels/${general}/messages$`),
      () => ({
        data: generalPage(url),
        included: { users, channels: [threadRecord], messages: [threadReplies[1]] },
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
  await page.routeWebSocket(/\/api\/v1\/events$/, (ws) => {
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
export async function signInToWorld(page: Page): Promise<void> {
  const poll = lunch(await events(page));
  await page.route(/\/api\/v1\//, (route) => answer(route, poll));
  await page.goto("/");
  await page.getByLabel("Username").fill("kate");
  await page.getByLabel("Password").fill("hunter22");
  await page.getByRole("button", { name: "Sign in", exact: true }).click();
}
