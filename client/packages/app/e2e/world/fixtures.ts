import { uuid } from "../stubs";

/**
 * The world's records: its people, community, roles, channels, history, reactions, DM, and
 * invite, with the helpers that make and page them. The routes answer from these.
 */

export const me = uuid;
export const bob = "0190f0a0-0000-7000-8000-000000000002";
export const community = "0190f0a0-0000-7000-8000-000000000010";
/** Kate's bot, a member of the community. */
export const helper = "0190f0a0-0000-7000-8000-000000000003";
const everyoneRole = "0190f0a0-0000-7000-8000-000000000040";
export const deploymentAdministrator = "0190f0a0-0000-7000-8000-000000000042";
export const organiserRole = "0190f0a0-0000-7000-8000-000000000041";
/** The community's one custom emoji, and its picture. */
export const customEmojiId = "0190f0a0-0000-7000-8000-0000000000e1";
export const customEmojiName = "partyparrot";
export const customEmojiIcon = "0190f0a0-0000-7000-8000-0000000000e2";
/** A one-pixel PNG, which stands for every emoji's picture. */
const PIXEL_PNG =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
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
export const roles = [
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
      "manageCustomEmoji",
      "banMembers",
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

export const minutesAgo = (minutes: number) =>
  new Date(Date.now() - minutes * 60_000).toISOString();
/** An id for a record the world makes during a test, each one new. */
let made = 0;
export const freshId = () =>
  `0190f0a0-0000-7000-8000-00000000f${(made++).toString(16).padStart(3, "0")}`;
/** An icon record whose picture is the one-pixel PNG. */
export const iconRecord = (id: string) => ({ id, mimeType: "image/png", downloadUrl: PIXEL_PNG });

export const user = (id: string, name: string, displayName: string | null) => ({
  id,
  onlineStatus: "online",
  name,
  icon: null,
  displayName,
  pronouns: null,
  bio: null,
  status: null,
  bot: false,
  system: false,
  botOwner: null as string | null,
  botPublic: false,
});

/** A member of the community beyond its member sample, whom only a search finds. */
export const farMember = user("0190f0a0-0000-7000-8000-000000000004", "dana", "Dana From Far Away");

export const users = [
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

export const channels = [
  channel(general, "general", "text"),
  channel(lounge, "Lounge", "voice", { sortIndex: 1 }),
  channel(roadmap, "roadmap", "text", { parentCategory: planning }),
  channel(ideas, "ideas", "text", { parentCategory: planning, sortIndex: 1 }),
];

// Ids grow with time, as UUIDv7 ones do: a window is ordered by them.
export const messageId = (n: number) => `0190f0a0-0000-7000-8001-${String(n).padStart(12, "0")}`;

export const message = (
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
  linkedMessages: [],
  kind: "standard",
  poll: null,
  thread: null,
  echoOf: null,
  content,
  attachments: [],
  mentions: { users: [], roles: [], everyone: false },
  ...extra,
});

/**
 * The organiser's role as a `role` update event would carry it with `extra` permissions added,
 * for a test that needs the caller to hold one the world's organiser lacks (Manage messages).
 */
export function organiserRoleWith(extra: readonly string[]): Record<string, unknown> {
  const organiser = roles.find((r) => r.id === organiserRole);
  return {
    serverEvent: "role",
    type: "update",
    id: organiserRole,
    permissions: [...(organiser?.permissions ?? []), ...extra],
  };
}

/** A share page named like a gif, which is a page; its preview's picture is the gif. */
export const gifPageLink = "https://tenor.com/view/ghost-12345.gif";
/** A link to a picture that no longer exists, which the tests answer with nothing. */
export const missingPictureLink = "https://pictures.example.com/missing-picture.png";
/** Where the missing picture's link is posted in #general. */
const missingPictureMessage = 199;
export const missingPictureMessageId = messageId(missingPictureMessage);

/** The thread's starter, in #general. */
export const starterText = "Who is bringing snacks on Saturday?";
/** A message of the caller's own, which offers editing and deleting. */
export const ownText = "I can bring the lemonade and some chairs.";

export const threadRecord = channel(thread, "", "thread", {
  parentChannel: general,
  starterMessage: messageId(203),
  replyCount: 2,
  lastReplyAt: minutesAgo(20),
});

export const threadReplies = [
  message(210, bob, "Crisps and a fruit platter from me.", 25, { channelId: thread }),
  message(211, me, "Perfect, thank you!", 20, { channelId: thread }),
];

/** The question of Bob's poll, the newest message in #general. */
export const pollQuestion = "Where should we have lunch?";

// Newest first, as the server answers.
export const generalMessages = [
  message(207, bob, "", 3, { kind: "poll", poll: lunchPoll }),
  message(206, me, ownText, 5),
  message(205, bob, "", 20, { kind: "threadEcho", echoOf: messageId(211) }),
  message(204, bob, "Sounds good. See everyone at ten, and bring a jumper in case it rains.", 40),
  message(203, me, starterText, 60, { thread }),
  message(202, bob, `Morning all! <:${customEmojiId}>`, 90),
  message(201, me, "Welcome to the family server.", 120),
  // A share page named like a gif, whose preview carries the gif itself; and, by the caller,
  // so Bob's messages make one run for the blocking tests, a link to a picture that is gone.
  message(200, bob, gifPageLink, 130, {
    linkPreviews: [
      {
        url: gifPageLink,
        title: "Ghost GIF",
        description: "Click to view the GIF",
        siteName: "Tenor",
        imageUrl: PIXEL_PNG,
        imageWidth: 1,
        imageHeight: 1,
        themeColor: null,
        video: null,
      },
    ],
  }),
  message(missingPictureMessage, me, missingPictureLink, 140),
  // Older history, enough for several pages, so reading back through it can be exercised.
  ...Array.from({ length: 120 }, (_, i) =>
    message(120 - i, i % 2 === 0 ? me : bob, `Older message ${String(120 - i)}`, 180 + i * 10),
  ),
];

/** The last message in #general the caller has read; everything after it is new. */
export const lastReadText =
  "Sounds good. See everyone at ten, and bring a jumper in case it rains.";

/** #roadmap holds unread messages, two of which tag the caller. */
export const communityReadStates = [
  { channel: general, lastRead: messageId(204), lastMessage: messageId(207), mentions: 0 },
  { channel: lounge, lastRead: messageId(1), lastMessage: null, mentions: 0 },
  { channel: roadmap, lastRead: messageId(1), lastMessage: messageId(230), mentions: 2 },
  { channel: ideas, lastRead: messageId(1), lastMessage: null, mentions: 0 },
];

/** Bob's "Sounds good" message, which has a crowd of reactions. */
export const reactedText = lastReadText;
export const reactedId = messageId(204);

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
export const reactionSummaries = [
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
export function generalPage(url: URL) {
  const limit = Number(url.searchParams.get("limit") ?? "50");
  const before = url.searchParams.get("before");
  const start = before === null ? 0 : generalMessages.findIndex((m) => m.id === before) + 1;
  return generalMessages.slice(start, start + limit);
}

export const dmRecord = {
  ...channel(dm, "", "dm"),
  community: null,
  recipients: [me, bob],
};

/** Bob's message in the DM, which tags the caller. */
export const dmMessages = [
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
export function searchMessages(url: URL) {
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
