import { describe, expect, it, vi } from "vitest";
import {
  RecordStore,
  TEMPLATES,
  WINDOW_MAX_MESSAGES,
  groupChannels,
  type Category,
  type Channel,
  type Community,
  type Invite,
  type Message,
  type Poll,
  type User,
} from "../src";

/** UUIDv7-shaped ids whose lexical order is their numeric order. */
function id(n: number): string {
  return `0190f0a0-0000-7000-8000-${n.toString(16).padStart(12, "0")}`;
}

const me: User = {
  id: id(1),
  name: "kate",
  icon: null,
  onlineStatus: "online",
  bot: false,
  system: false,
  botPublic: false,
};
const bob: User = {
  id: id(2),
  name: "bob",
  icon: null,
  onlineStatus: "offline",
  bot: false,
  system: false,
  botPublic: false,
};
const aspen: Community = { id: id(10), name: "Aspen", icon: null };
const birch: Community = { id: id(11), name: "Birch", icon: null };
const general: Channel = {
  id: id(20),
  community: aspen.id,
  parentCategory: null,
  name: "general",
  sortIndex: 0,
  ty: "text",
  replyCount: 0,
  recipients: [],
};
const dev: Channel = {
  id: id(21),
  community: aspen.id,
  parentCategory: id(30),
  name: "dev",
  sortIndex: 1,
  ty: "text",
  replyCount: 0,
  recipients: [],
};
const work: Category = { id: id(30), community: aspen.id, name: "Work", sortIndex: 0 };

function message(n: number, channelId = general.id, author = me.id): Message {
  return {
    id: id(1000 + n),
    channelId,
    author,
    content: `message ${String(n)}`,
    timestamp: "2026-09-25T12:00:00Z",
    editedAt: null,
    attachments: [],
    linkPreviews: [],
    kind: "standard",
    poll: null,
    mentions: { users: [], roles: [], everyone: false },
    linkedMessages: [],
    alteredBy: [],
  };
}

function bootstrapped(): RecordStore {
  const store = new RecordStore();
  store.setBootstrap(me, [aspen, birch], {
    channels: [general, dev],
    categories: [work],
    users: [me, bob],
    userCommunities: [
      { community: aspen.id, user: me.id, sortIndex: 0, roles: [] },
      { community: aspen.id, user: bob.id, sortIndex: 0, roles: [] },
      { community: birch.id, user: me.id, sortIndex: 0, roles: [] },
    ],
  });
  return store;
}

describe("RecordStore bootstrap", () => {
  it("indexes communities, channels, categories, and members", () => {
    const store = bootstrapped();
    expect(store.me()).toEqual(me);
    expect(store.communities().map((c) => c.name)).toEqual(["Aspen", "Birch"]);
    expect(store.channels(aspen.id).map((c) => c.name)).toEqual(["general", "dev"]);
    expect(store.categories(aspen.id)).toEqual([work]);
    expect(store.memberIds(aspen.id)).toEqual([me.id, bob.id]);
    expect(store.memberIds(birch.id)).toEqual([me.id]);
  });

  it("returns the same reference until something relevant changes", () => {
    const store = bootstrapped();
    const channels = store.channels(aspen.id);
    expect(store.channels(aspen.id)).toBe(channels);
    store.applyEvent({ serverEvent: "user", type: "update", id: bob.id, name: "robert" });
    expect(store.channels(aspen.id)).toBe(channels);
    store.applyEvent({ serverEvent: "channel", type: "update", id: dev.id, name: "development" });
    expect(store.channels(aspen.id)).not.toBe(channels);
    expect(store.channel(dev.id)?.name).toBe("development");
  });

  it("reconciles a second bootstrap, dropping what the server no longer lists", () => {
    const store = bootstrapped();
    store.replaceWindow(general.id, [message(1)], { hasOlder: false, atLatest: true });
    store.setBootstrap(me, [aspen], {
      channels: [general],
      categories: [],
      users: [me],
      userCommunities: [{ community: aspen.id, user: me.id, sortIndex: 0, roles: [] }],
    });
    expect(store.communities()).toEqual([aspen]);
    expect(store.community(birch.id)).toBeUndefined();
    expect(store.channels(aspen.id)).toEqual([general]);
    expect(store.categories(aspen.id)).toEqual([]);
    expect(store.memberIds(aspen.id)).toEqual([me.id]);
    // History may have gaps after a resync, so windows are dropped and reloaded on demand.
    expect(store.messages(general.id)).toBeUndefined();
    expect(store.message(message(1).id)).toBeUndefined();
    // Users are kept: a stale record is harmless and cheaper than refetching every author.
    expect(store.user(bob.id)).toEqual(bob);
  });
});

describe("RecordStore events", () => {
  it("applies updates as merge patches and notifies exactly the touched topics", () => {
    const store = bootstrapped();
    const onAspen = vi.fn();
    const onList = vi.fn();
    const onBirch = vi.fn();
    store.subscribe(`community:${aspen.id}`, onAspen);
    store.subscribe("communities", onList);
    store.subscribe(`community:${birch.id}`, onBirch);
    store.applyEvent({ serverEvent: "community", type: "update", id: aspen.id, icon: id(99) });
    expect(store.community(aspen.id)).toEqual({ ...aspen, icon: id(99) });
    store.applyEvent({ serverEvent: "community", type: "update", id: aspen.id, icon: null });
    expect(store.community(aspen.id)?.icon).toBeNull();
    expect(onAspen).toHaveBeenCalledTimes(2);
    expect(onList).toHaveBeenCalledTimes(2);
    expect(onBirch).not.toHaveBeenCalled();
  });

  it("ignores updates for records it does not hold", () => {
    const store = bootstrapped();
    store.applyEvent({ serverEvent: "channel", type: "update", id: id(77), name: "ghost" });
    expect(store.channel(id(77))).toBeUndefined();
  });

  it("cascades a community delete to its channels, categories, members, and messages", () => {
    const store = bootstrapped();
    store.replaceWindow(general.id, [message(1)], { hasOlder: false, atLatest: true });
    const onMessages = vi.fn();
    store.subscribe(`messages:${general.id}`, onMessages);
    store.applyEvent({ serverEvent: "community", type: "delete", id: aspen.id });
    expect(store.communities()).toEqual([birch]);
    expect(store.channel(general.id)).toBeUndefined();
    expect(store.category(work.id)).toBeUndefined();
    expect(store.memberIds(aspen.id)).toEqual([]);
    expect(store.messages(general.id)).toBeUndefined();
    expect(store.message(message(1).id)).toBeUndefined();
    expect(onMessages).toHaveBeenCalledTimes(1);
  });

  it("moves a deleted category's channels to the top level", () => {
    const store = bootstrapped();
    store.applyEvent({ serverEvent: "category", type: "delete", id: work.id });
    expect(store.channel(dev.id)?.parentCategory).toBeNull();
  });

  it("tracks the caller's own memberships separately from the member sample", () => {
    const store = bootstrapped();
    const cedar: Community = { id: id(12), name: "Cedar", icon: null };
    store.applyEvent({ serverEvent: "community", type: "create", ...cedar });
    expect(store.communities()).toEqual([aspen, birch]);
    store.applyEvent({
      serverEvent: "userCommunity",
      type: "create",
      community: cedar.id,
      user: me.id,
      sortIndex: 5,
      roles: [],
    });
    expect(store.communities()).toEqual([aspen, birch, cedar]);
    store.applyEvent({
      serverEvent: "userCommunity",
      type: "delete",
      community: aspen.id,
      user: me.id,
    });
    // Leaving takes the whole community away, its channels and member sample with it.
    expect(store.communities()).toEqual([birch, cedar]);
    expect(store.community(aspen.id)).toBeUndefined();
    expect(store.memberIds(aspen.id)).toEqual([]);
    expect(store.channels(aspen.id)).toEqual([]);
  });

  it("regroups member records when a member's status changes", () => {
    const store = bootstrapped();
    const onMembers = vi.fn();
    store.subscribe(`members:${aspen.id}`, onMembers);
    const before = store.members(aspen.id);
    expect(before.map((u) => u.name)).toEqual(["kate", "bob"]);
    store.applyStatuses([{ id: bob.id, onlineStatus: "online" }]);
    expect(onMembers).toHaveBeenCalledTimes(1);
    const after = store.members(aspen.id);
    expect(after).not.toBe(before);
    expect(after[1]?.onlineStatus).toBe("online");
    // A user who is not a member of Aspen does not touch its list.
    store.applyEvent({
      serverEvent: "user",
      type: "create",
      id: id(5),
      name: "eve",
      icon: null,
      onlineStatus: "online",
      bot: false,
      system: false,
      botPublic: false,
    });
    expect(onMembers).toHaveBeenCalledTimes(1);
  });

  it("carries the edited-at stamp of an update event", () => {
    const store = bootstrapped();
    store.replaceWindow(general.id, [message(1)], { hasOlder: false, atLatest: true });
    store.applyEvent({
      serverEvent: "message",
      type: "update",
      id: message(1).id,
      content: "changed",
      editedAt: "2026-09-25T12:30:00Z",
    });
    expect(store.message(message(1).id)).toMatchObject({
      content: "changed",
      editedAt: "2026-09-25T12:30:00Z",
    });
  });

  it("patches online status and link previews", () => {
    const store = bootstrapped();
    store.replaceWindow(general.id, [message(1)], { hasOlder: false, atLatest: true });
    store.applyStatuses([{ id: bob.id, onlineStatus: "online" }]);
    expect(store.user(bob.id)?.onlineStatus).toBe("online");
    expect(store.presenceCandidates()).toEqual(expect.arrayContaining([me.id, bob.id]));
    // Link previews arrive as an ordinary update once the server has fetched them.
    store.applyEvent({
      serverEvent: "message",
      type: "update",
      id: message(1).id,
      linkPreviews: [{ url: "https://example.org", title: "Example" }],
    });
    expect(store.message(message(1).id)).toMatchObject({
      content: "message 1",
      linkPreviews: [{ url: "https://example.org", title: "Example" }],
    });
  });

  it("collects reactions per message and emoji", () => {
    const store = bootstrapped();
    const target = message(1).id;
    store.applyEvent({
      serverEvent: "react",
      type: "create",
      messageId: target,
      emoji: "😁",
      userId: me.id,
    });
    store.applyEvent({
      serverEvent: "react",
      type: "create",
      messageId: target,
      emoji: "😁",
      userId: bob.id,
    });
    const first = store.reactions(target);
    expect(first.get("😁")).toEqual({ count: 2, me: true, users: [me.id, bob.id] });
    store.applyEvent({
      serverEvent: "react",
      type: "delete",
      messageId: target,
      emoji: "😁",
      userId: me.id,
    });
    expect(store.reactions(target)).not.toBe(first);
    expect(store.reactions(target).get("😁")).toEqual({ count: 1, me: false, users: [bob.id] });
    store.applyEvent({
      serverEvent: "react",
      type: "delete",
      messageId: target,
      emoji: "😁",
      userId: bob.id,
    });
    expect(store.reactions(target).size).toBe(0);
  });
});

describe("RecordStore reaction summaries", () => {
  it("installs a read's summaries, clearing messages it brought none for", () => {
    const store = bootstrapped();
    const [a, b] = [id(1501), id(1502)];
    store.setReactions([a], [{ messageId: a, emoji: "🎉", count: 1, me: false, users: [bob.id] }]);
    store.setReactions(
      [a, b],
      [
        { messageId: b, emoji: "👍", count: 9, me: true, users: [bob.id, me.id] },
        { messageId: b, emoji: "🎉", count: 2, me: false, users: [bob.id] },
      ],
    );
    expect(store.reactions(a).size).toBe(0);
    expect(Array.from(store.reactions(b).keys())).toEqual(["👍", "🎉"]);
  });

  it("counts someone beyond the named few, and the caller's own reaction once", () => {
    const store = bootstrapped();
    const target = id(1503);
    const others = [id(91), id(92), id(93), id(94)];
    store.setReactions(
      [target],
      [{ messageId: target, emoji: "👍", count: 4, me: false, users: others }],
    );
    const react = (userId: string, type: "create" | "delete") => {
      store.applyEvent({ serverEvent: "react", type, messageId: target, emoji: "👍", userId });
    };
    react(bob.id, "create");
    expect(store.reactions(target).get("👍")).toEqual({ count: 5, me: false, users: others });
    react(me.id, "create");
    react(me.id, "create");
    expect(store.reactions(target).get("👍")?.count).toBe(6);
    expect(store.reactions(target).get("👍")?.me).toBe(true);
    react(bob.id, "delete");
    react(me.id, "delete");
    react(me.id, "delete");
    expect(store.reactions(target).get("👍")).toEqual({ count: 4, me: false, users: others });
  });
});

describe("RecordStore message windows", () => {
  it("keeps ids sorted and appends new messages only to a window at the latest", () => {
    const store = bootstrapped();
    store.replaceWindow(general.id, [message(3), message(1), message(2)], {
      hasOlder: true,
      atLatest: true,
    });
    expect(store.messages(general.id)?.ids).toEqual([message(1).id, message(2).id, message(3).id]);
    store.applyEvent({ serverEvent: "message", type: "create", ...message(5) });
    store.applyEvent({ serverEvent: "message", type: "create", ...message(4) });
    expect(store.messages(general.id)?.ids).toEqual([1, 2, 3, 4, 5].map((n) => message(n).id));

    store.replaceWindow(general.id, [message(1), message(2)], { hasOlder: false, atLatest: false });
    store.applyEvent({ serverEvent: "message", type: "create", ...message(6) });
    expect(store.messages(general.id)?.ids).toEqual([message(1).id, message(2).id]);
    expect(store.message(message(6).id)).toBeDefined();
  });

  it("does not duplicate a message that was cached before its create event arrived", () => {
    const store = bootstrapped();
    store.replaceWindow(general.id, [], { hasOlder: false, atLatest: true });
    store.addMessage(message(1));
    store.applyEvent({ serverEvent: "message", type: "create", ...message(1) });
    expect(store.messages(general.id)?.ids).toEqual([message(1).id]);
  });

  it("keeps the streamed copy when a create response arrives after its events", () => {
    const store = bootstrapped();
    store.replaceWindow(general.id, [], { hasOlder: false, atLatest: true });
    store.applyEvent({ serverEvent: "message", type: "create", ...message(1) });
    store.applyEvent({
      serverEvent: "message",
      type: "update",
      id: message(1).id,
      linkPreviews: [{ url: "https://example.org", title: "Example" }],
    });
    store.addMessage(message(1));
    expect(store.message(message(1).id)?.linkPreviews).toEqual([
      { url: "https://example.org", title: "Example" },
    ]);
    expect(store.messages(general.id)?.ids).toEqual([message(1).id]);
  });

  it("prepends older pages and removes deleted messages", () => {
    const store = bootstrapped();
    store.replaceWindow(general.id, [message(5), message(6)], { hasOlder: true, atLatest: true });
    store.prependWindow(general.id, [message(4), message(3)], false);
    expect(store.messages(general.id)).toEqual({
      ids: [3, 4, 5, 6].map((n) => message(n).id),
      hasOlder: false,
      atLatest: true,
    });
    store.applyEvent({ serverEvent: "message", type: "delete", id: message(4).id });
    expect(store.messages(general.id)?.ids).toEqual([3, 5, 6].map((n) => message(n).id));
    expect(store.message(message(4).id)).toBeUndefined();
  });

  it("ignores messages for channels without a loaded window", () => {
    const store = bootstrapped();
    store.applyEvent({ serverEvent: "message", type: "create", ...message(1, dev.id) });
    expect(store.messages(dev.id)).toBeUndefined();
    expect(store.message(message(1, dev.id).id)).toBeDefined();
  });
});

describe("RecordStore invites", () => {
  const invite = (
    code: string,
    community = aspen.id,
    createdAt = "2026-09-25T12:00:00Z",
  ): Invite => ({
    code,
    community,
    createdBy: me.id,
    createdAt,
    expiresAt: null,
  });

  it("lists a community's invites newest first and replaces them on reload", () => {
    const store = bootstrapped();
    store.replaceInvites(aspen.id, [
      invite("old", aspen.id, "2026-09-24T12:00:00Z"),
      invite("new"),
    ]);
    expect(store.invites(aspen.id).map((i) => i.code)).toEqual(["new", "old"]);
    store.replaceInvites(aspen.id, [invite("only")]);
    expect(store.invites(aspen.id).map((i) => i.code)).toEqual(["only"]);
    expect(store.invite("old")).toBeUndefined();
  });

  it("applies invite events for whatever community they name, since the server routes them", () => {
    const store = bootstrapped();
    store.applyEvent({ serverEvent: "invite", type: "create", ...invite("mine") });
    store.applyEvent({ serverEvent: "invite", type: "create", ...invite("theirs", id(99)) });
    expect(store.invite("mine")).toBeDefined();
    expect(store.invite("theirs")).toBeDefined();
    store.applyEvent({
      serverEvent: "invite",
      type: "update",
      code: "mine",
      expiresAt: "2027-01-01T00:00:00Z",
    });
    expect(store.invite("mine")?.expiresAt).toBe("2027-01-01T00:00:00Z");
    store.applyEvent({ serverEvent: "invite", type: "delete", code: "mine" });
    expect(store.invites(aspen.id)).toEqual([]);
  });

  it("drops a community's invites when the community is deleted", () => {
    const store = bootstrapped();
    store.upsertInvite(invite("x"));
    store.applyEvent({ serverEvent: "community", type: "delete", id: aspen.id });
    expect(store.invite("x")).toBeUndefined();
  });
});

describe("groupChannels", () => {
  it("files channels under their categories and the rest at the top level", () => {
    const orphan: Channel = { ...dev, id: id(22), parentCategory: id(31), name: "orphan" };
    const grouped = groupChannels([general, dev, orphan], [work]);
    expect(grouped.topLevel.map((c) => c.name)).toEqual(["general", "orphan"]);
    expect(grouped.byCategory.get(work.id)?.map((c) => c.name)).toEqual(["dev"]);
  });
});

function poll(n: number, anonymous = false): Poll {
  return {
    id: id(3000 + n),
    channelId: general.id,
    messageId: id(1000 + n),
    createdBy: me.id,
    createdAt: "2026-09-25T12:00:00Z",
    closesAt: "2026-09-25T13:00:00Z",
    closedAt: null,
    question: "Lunch?",
    options: [{ label: "Pizza", emoji: "🍕" }, { label: "Sushi" }],
    multipleChoice: false,
    allowWriteIns: true,
    writeIns: [],
    anonymous,
    results: anonymous
      ? [{ count: 0 }, { count: 0 }]
      : [
          { count: 0, voters: [] },
          { count: 0, voters: [] },
        ],
  };
}

describe("RecordStore community order", () => {
  it("orders communities by the caller's membership index, then name", () => {
    const store = bootstrapped();
    expect(store.communities()).toEqual([aspen, birch]);
    store.applyEvent({
      serverEvent: "userCommunity",
      type: "update",
      community: aspen.id,
      user: me.id,
      sortIndex: 3,
    });
    expect(store.communities()).toEqual([birch, aspen]);
    // Another member's arrangement is theirs alone.
    store.applyEvent({
      serverEvent: "userCommunity",
      type: "update",
      community: birch.id,
      user: bob.id,
      sortIndex: 9,
    });
    expect(store.communities()).toEqual([birch, aspen]);
    const listener = vi.fn();
    store.subscribe("communities", listener);
    store.setCommunityOrder(birch.id, 4);
    expect(store.communities()).toEqual([aspen, birch]);
    expect(listener).toHaveBeenCalledTimes(1);
  });
});

describe("RecordStore window bounds", () => {
  const page = (from: number, count: number) =>
    Array.from({ length: count }, (_, i) => message(from + i));

  it("drops the newest messages when older ones push the window past its cap", () => {
    const store = bootstrapped();
    // Two halves of the window and a page more, which it cannot all hold.
    const half = WINDOW_MAX_MESSAGES / 2 + 10;
    store.replaceWindow(general.id, page(100 + half, half), { hasOlder: true, atLatest: true });
    store.prependWindow(general.id, page(100, half), true);
    const window = store.messages(general.id);
    const kept = 100 + WINDOW_MAX_MESSAGES - 1;
    expect(window?.ids).toHaveLength(WINDOW_MAX_MESSAGES);
    expect(window?.ids[0]).toBe(message(100).id);
    expect(window?.ids.at(-1)).toBe(message(kept).id);
    expect(window?.atLatest).toBe(false);
    expect(store.message(message(kept + 1).id)).toBeUndefined();
    expect(store.message(message(kept).id)).toBeDefined();
  });

  it("drops the oldest messages when newer ones push the window past its cap", () => {
    const store = bootstrapped();
    const half = WINDOW_MAX_MESSAGES / 2 + 10;
    store.replaceWindow(general.id, page(100, half), { hasOlder: false, atLatest: false });
    store.appendWindow(general.id, page(100 + half, half), true);
    const window = store.messages(general.id);
    const newest = 100 + 2 * half - 1;
    expect(window?.ids).toHaveLength(WINDOW_MAX_MESSAGES);
    expect(window?.ids[0]).toBe(message(newest - WINDOW_MAX_MESSAGES + 1).id);
    expect(window?.ids.at(-1)).toBe(message(newest).id);
    expect(window).toMatchObject({ hasOlder: true, atLatest: true });
    expect(store.message(message(100).id)).toBeUndefined();
  });

  it("lets a live window grow past the cap, dropping the oldest only once it is twice the cap", () => {
    const store = bootstrapped();
    store.replaceWindow(general.id, page(100, WINDOW_MAX_MESSAGES), {
      hasOlder: false,
      atLatest: true,
    });
    // A reader may be on the oldest of a full window: one arrival drops nothing.
    store.applyEvent({ serverEvent: "message", type: "create", ...message(900) });
    let window = store.messages(general.id);
    expect(window?.ids).toHaveLength(WINDOW_MAX_MESSAGES + 1);
    expect(window?.ids[0]).toBe(message(100).id);
    expect(window?.hasOlder).toBe(false);
    // Twice the cap, and the window is back to the cap, its oldest gone.
    for (let n = 901; n <= 900 + WINDOW_MAX_MESSAGES; n++) {
      store.applyEvent({ serverEvent: "message", type: "create", ...message(n) });
    }
    // Three hundred and one arrived; the newest three hundred stay.
    window = store.messages(general.id);
    expect(window?.ids).toHaveLength(WINDOW_MAX_MESSAGES);
    expect(window?.ids[0]).toBe(message(901).id);
    expect(window?.hasOlder).toBe(true);
    expect(store.message(message(100).id)).toBeUndefined();
  });
});

describe("RecordStore read states", () => {
  const at = (n: number) => id(1000 + n);
  const readOf = (store: RecordStore) => store.readState(general.id);

  it("says a channel is unread while someone else's message is after the position", () => {
    const store = bootstrapped();
    store.ingest({
      readStates: [
        { channel: general.id, lastRead: at(100), lastMessage: at(101), mentions: 0 },
        { channel: dev.id, lastRead: at(100), lastMessage: at(90), mentions: 0 },
      ],
    });
    expect(store.unread(general.id)).toBe(true);
    expect(store.unread(dev.id)).toBe(false);
    expect(Array.from(store.unreadPlaces())).toEqual([aspen.id]);
    store.setLastRead(general.id, at(101));
    expect(store.unread(general.id)).toBe(false);
    expect(store.unreadPlaces().size).toBe(0);
  });

  it("follows messages as they arrive: someone else's is new, the caller's own is read", () => {
    const store = bootstrapped();
    store.ingest({
      readStates: [{ channel: general.id, lastRead: at(100), lastMessage: null, mentions: 0 }],
    });
    store.applyEvent({
      serverEvent: "message",
      type: "create",
      ...message(110, general.id, bob.id),
    });
    expect(readOf(store)).toEqual({
      channel: general.id,
      lastRead: at(100),
      lastMessage: at(110),
      mentions: 0,
    });
    expect(store.unread(general.id)).toBe(true);
    store.applyEvent({
      serverEvent: "message",
      type: "create",
      ...message(111, general.id, me.id),
    });
    expect(readOf(store)?.lastRead).toBe(at(111));
    expect(store.unread(general.id)).toBe(false);
    // A channel with no read state yet is unread from its start.
    store.applyEvent({ serverEvent: "message", type: "create", ...message(112, dev.id, bob.id) });
    expect(store.readState(dev.id)).toEqual({
      channel: dev.id,
      lastRead: "",
      lastMessage: at(112),
      mentions: 0,
    });
    expect(store.unread(dev.id)).toBe(true);
  });

  it("moves forward when another device reads, never back", () => {
    const store = bootstrapped();
    store.ingest({
      readStates: [{ channel: general.id, lastRead: at(100), lastMessage: at(105), mentions: 0 }],
    });
    store.applyEvent({ serverEvent: "channelRead", channel: general.id, lastRead: at(99) });
    expect(readOf(store)?.lastRead).toBe(at(100));
    store.applyEvent({ serverEvent: "channelRead", channel: general.id, lastRead: at(105) });
    expect(readOf(store)?.lastRead).toBe(at(105));
    expect(store.channelsLastMessaged(at(105))).toEqual([general.id]);
  });

  it("keeps no read state for threads, nor for a channel once it is gone", () => {
    const store = bootstrapped();
    const thread: Channel = {
      ...general,
      id: id(29),
      ty: "thread",
      parentChannel: general.id,
      starterMessage: at(100),
    };
    store.ingest({
      channels: [thread],
      readStates: [{ channel: general.id, lastRead: at(100), lastMessage: at(101), mentions: 0 }],
    });
    store.applyEvent({
      serverEvent: "message",
      type: "create",
      ...message(120, thread.id, bob.id),
    });
    expect(store.readState(thread.id)).toBeUndefined();
    store.applyEvent({ serverEvent: "channel", type: "delete", id: general.id });
    expect(store.readState(general.id)).toBeUndefined();
    expect(store.unreadPlaces().size).toBe(0);
  });
});

describe("RecordStore mutes", () => {
  const unreadGeneral = {
    channel: general.id,
    lastRead: id(1100),
    lastMessage: id(1101),
    mentions: 0,
  };

  it("keeps a muted channel's unread out of its community's mark", () => {
    const store = bootstrapped();
    store.ingest({ readStates: [unreadGeneral] });
    expect(Array.from(store.unreadPlaces())).toEqual([aspen.id]);
    store.applyEvent({ serverEvent: "channelMuteChanged", channel: general.id, muted: true });
    expect(store.mute(general.id)).toEqual({ channel: general.id, until: null });
    expect(store.unreadPlaces().size).toBe(0);
    // Still unread underneath, for when the mute ends.
    expect(store.unread(general.id)).toBe(true);
    store.applyEvent({ serverEvent: "channelMuteChanged", channel: general.id, muted: false });
    expect(store.mute(general.id)).toBeUndefined();
    expect(Array.from(store.unreadPlaces())).toEqual([aspen.id]);
  });

  it("ends timed mutes when their time is up, and names the next to end", () => {
    const store = bootstrapped();
    store.ingest({
      channelMutes: [
        { channel: general.id, until: "2026-09-28T12:00:00Z" },
        { channel: dev.id, until: "2026-09-28T13:00:00Z" },
      ],
    });
    expect(store.nextMuteEnd()).toBe(Date.parse("2026-09-28T12:00:00Z"));
    store.expireMutes(Date.parse("2026-09-28T12:00:00Z"));
    expect(store.mute(general.id)).toBeUndefined();
    expect(store.mute(dev.id)).toBeDefined();
    expect(store.nextMuteEnd()).toBe(Date.parse("2026-09-28T13:00:00Z"));
  });

  it("replaces every mute with what a bootstrap read", () => {
    const store = bootstrapped();
    store.ingest({ channelMutes: [{ channel: general.id, until: null }] });
    store.replaceMutes([{ channel: dev.id, until: null }]);
    expect(store.mute(general.id)).toBeUndefined();
    expect(store.mute(dev.id)).toEqual({ channel: dev.id, until: null });
  });
});

describe("RecordStore collapsed categories", () => {
  it("follows the caller's devices and replaces what a bootstrap read", () => {
    const store = bootstrapped();
    store.ingest({ categoryCollapses: [{ category: work.id }] });
    expect(store.collapsed(work.id)).toBe(true);
    store.applyEvent({
      serverEvent: "categoryCollapseChanged",
      category: work.id,
      collapsed: false,
    });
    expect(store.collapsed(work.id)).toBe(false);
    store.applyEvent({
      serverEvent: "categoryCollapseChanged",
      category: work.id,
      collapsed: true,
    });
    store.replaceCollapsed([]);
    expect(store.collapsed(work.id)).toBe(false);
  });

  it("keeps an unread channel in view under a collapsed category, unless it is muted", () => {
    const store = bootstrapped();
    store.ingest({
      readStates: [{ channel: dev.id, lastRead: id(1100), lastMessage: id(1101), mentions: 0 }],
    });
    expect(store.shownWhenCollapsed(dev.id)).toBe(true);
    expect(store.shownWhenCollapsed(general.id)).toBe(false);
    store.applyEvent({ serverEvent: "channelMuteChanged", channel: dev.id, muted: true });
    expect(store.shownWhenCollapsed(dev.id)).toBe(false);
  });
});

describe("RecordStore poll write-ins", () => {
  it("holds the caller's own write-ins and follows the answers others add and remove", () => {
    const store = bootstrapped();
    const lunch = poll(1, true);
    store.ingest({
      polls: [lunch],
      pollVotes: [{ poll: lunch.id, option: 2 }],
      ownWriteIns: [{ poll: lunch.id, option: 2 }],
    });
    expect(Array.from(store.myWriteIns(lunch.id))).toEqual([2]);
    store.applyEvent({
      serverEvent: "poll",
      type: "update",
      id: lunch.id,
      writeIns: [{ label: "Tacos" }, null, { label: "Curry" }],
      results: [{ count: 0 }, { count: 0 }, { count: 1 }, { count: 0 }, { count: 1 }],
    });
    expect(store.poll(lunch.id)?.writeIns).toEqual([{ label: "Tacos" }, null, { label: "Curry" }]);
    // Someone else removed the caller's answer: it, and their vote for it, are gone.
    store.applyEvent({
      serverEvent: "poll",
      type: "update",
      id: lunch.id,
      writeIns: [null, null, { label: "Curry" }],
      results: [{ count: 0 }, { count: 0 }, { count: 0 }, { count: 0 }, { count: 1 }],
    });
    expect(Array.from(store.myWriteIns(lunch.id))).toEqual([]);
    expect(Array.from(store.myVotes(lunch.id))).toEqual([]);
    // A later read says the caller has none on the poll.
    store.setMyWriteIn(lunch.id, 4, true);
    store.ingest({ polls: [lunch], pollVotes: [], ownWriteIns: [] });
    expect(Array.from(store.myWriteIns(lunch.id))).toEqual([]);
  });
});

describe("RecordStore polls", () => {
  it("ingests polls with the caller's votes and follows tally events", () => {
    const store = bootstrapped();
    const lunch = poll(1, true);
    store.ingest({ polls: [lunch], pollVotes: [{ poll: lunch.id, option: 1 }] });
    expect(store.poll(lunch.id)).toEqual(lunch);
    expect(Array.from(store.myVotes(lunch.id))).toEqual([1]);

    const listener = vi.fn();
    store.subscribe(`poll:${lunch.id}`, listener);
    store.applyEvent({
      serverEvent: "poll",
      type: "update",
      id: lunch.id,
      results: [{ count: 0 }, { count: 1 }],
    });
    expect(store.poll(lunch.id)?.results[1]?.count).toBe(1);
    expect(store.poll(lunch.id)?.closedAt).toBeNull();
    expect(listener).toHaveBeenCalledTimes(1);

    store.applyEvent({
      serverEvent: "poll",
      type: "update",
      id: lunch.id,
      closedAt: "2026-09-25T13:00:00Z",
    });
    expect(store.poll(lunch.id)?.closedAt).toBe("2026-09-25T13:00:00Z");
  });

  it("tracks the caller's own votes, one at a time on a single-choice poll", () => {
    const store = bootstrapped();
    const lunch = poll(2);
    store.addPoll(lunch);
    expect(store.myVotes(lunch.id).size).toBe(0);
    store.setMyVote(lunch.id, 0, true);
    store.setMyVote(lunch.id, 1, true);
    expect(Array.from(store.myVotes(lunch.id))).toEqual([1]);
    store.setMyVote(lunch.id, 1, false);
    expect(store.myVotes(lunch.id).size).toBe(0);

    const many = { ...poll(3), multipleChoice: true };
    store.addPoll(many);
    store.setMyVote(many.id, 0, true);
    store.setMyVote(many.id, 1, true);
    expect(Array.from(store.myVotes(many.id))).toEqual([0, 1]);
  });

  it("keeps a streamed poll over the create response, and drops it with its message", () => {
    const store = bootstrapped();
    const lunch = poll(4);
    const voted = {
      ...lunch,
      results: [
        { count: 1, voters: [bob.id] },
        { count: 0, voters: [] },
      ],
    };
    store.applyEvent({ serverEvent: "poll", type: "create", ...voted });
    store.addPoll(lunch);
    expect(store.poll(lunch.id)).toEqual(voted);

    store.replaceWindow(general.id, [{ ...message(4), kind: "poll", poll: lunch.id }], {
      hasOlder: false,
      atLatest: true,
    });
    store.applyEvent({ serverEvent: "message", type: "delete", id: id(1004) });
    expect(store.poll(lunch.id)).toBeUndefined();
  });
});

describe("RecordStore voice", () => {
  const session = {
    id: id(9000),
    channel: general.id,
    voiceServer: id(9500),
    createdAt: "2026-09-26T00:00:00Z",
  };
  const participant = (user: string, joinedAt: string) => ({
    session: session.id,
    user,
    channel: general.id,
    joinedAt,
    muted: false,
    deafened: false,
    sharingScreen: false,
  });

  it("follows a call from sideload through join, speaking, mute, leave, and end", () => {
    let now = 1_000;
    const store = new RecordStore({ now: () => now });
    store.setBootstrap(me, [aspen], {
      channels: [general],
      categories: [],
      users: [me],
      userCommunities: [{ community: aspen.id, user: me.id, sortIndex: 0, roles: [] }],
      voiceSessions: [session],
      voiceParticipants: [participant(me.id, "2026-09-26T00:00:01Z")],
    });
    expect(store.channelVoice(general.id).session).toEqual(session);
    expect(store.channelVoice(general.id).participants.map((p) => p.user)).toEqual([me.id]);
    const listener = vi.fn();
    store.subscribe(`voice:${general.id}`, listener);
    store.applyEvent({
      serverEvent: "voiceParticipant",
      type: "create",
      ...participant(bob.id, "2026-09-26T00:00:02Z"),
    });
    expect(store.channelVoice(general.id).participants.map((p) => p.user)).toEqual([me.id, bob.id]);
    now = 5_000;
    store.applyEvent({
      serverEvent: "voiceSpeaking",
      channel: general.id,
      user: bob.id,
      speaking: true,
    });
    const speaking = store.channelVoice(general.id).participants.find((p) => p.user === bob.id);
    expect(speaking).toMatchObject({ speaking: true, lastSpokeAt: 5_000 });
    store.applyEvent({
      serverEvent: "voiceSpeaking",
      channel: general.id,
      user: bob.id,
      speaking: false,
    });
    expect(
      store.channelVoice(general.id).participants.find((p) => p.user === bob.id),
    ).toMatchObject({ speaking: false, lastSpokeAt: 5_000 });
    store.applyEvent({
      serverEvent: "voiceParticipant",
      type: "update",
      session: session.id,
      user: me.id,
      muted: true,
    });
    expect(store.channelVoice(general.id).participants[0]).toMatchObject({
      muted: true,
      sharingScreen: false,
    });
    store.applyEvent({
      serverEvent: "voiceParticipant",
      type: "update",
      session: session.id,
      user: me.id,
      sharingScreen: true,
    });
    expect(store.channelVoice(general.id).participants[0]).toMatchObject({
      muted: true,
      sharingScreen: true,
    });
    store.applyEvent({
      serverEvent: "voiceParticipant",
      type: "delete",
      session: session.id,
      user: bob.id,
    });
    expect(store.channelVoice(general.id).participants).toHaveLength(1);
    store.applyEvent({
      serverEvent: "voiceSessionEnded",
      id: session.id,
      channel: general.id,
      reason: "empty",
    });
    store.applyEvent({ serverEvent: "voiceSession", type: "delete", id: session.id });
    expect(store.channelVoice(general.id)).toEqual({ session: null, participants: [], rings: [] });
    expect(listener).toHaveBeenCalled();
  });

  it("keeps who a call rings until they join or decline, or the call ends", () => {
    const store = new RecordStore();
    store.setBootstrap(me, [aspen], {
      channels: [general],
      categories: [],
      users: [me],
      userCommunities: [{ community: aspen.id, user: me.id, sortIndex: 0, roles: [] }],
      voiceSessions: [session],
      voiceParticipants: [],
    });
    const other = id(42);
    const ring = (user: string) => ({
      session: session.id,
      user,
      channel: general.id,
      caller: other,
      until: "2026-09-26T00:00:15Z",
    });
    const rings = vi.fn();
    store.subscribe("rings", rings);
    store.applyEvent({ serverEvent: "voiceRing", type: "create", ...ring(me.id) });
    store.applyEvent({ serverEvent: "voiceRing", type: "create", ...ring(id(43)) });
    expect(store.myRings()).toEqual([ring(me.id)]);
    expect(store.channelVoice(general.id).rings).toHaveLength(2);
    expect(rings).toHaveBeenCalled();
    store.applyEvent({
      serverEvent: "voiceRing",
      type: "delete",
      session: session.id,
      user: me.id,
    });
    expect(store.myRings()).toEqual([]);
    expect(store.channelVoice(general.id).rings).toEqual([ring(id(43))]);
    // The call's end ends every ring of it.
    store.applyEvent({ serverEvent: "voiceSession", type: "delete", id: session.id });
    expect(store.channelVoice(general.id).rings).toEqual([]);
  });

  it("drops a call with its channel", () => {
    const store = bootstrapped();
    store.applyEvent({ serverEvent: "voiceSession", type: "create", ...session });
    store.applyEvent({ serverEvent: "channel", type: "delete", id: general.id });
    expect(store.channelVoice(general.id).session).toBeNull();
  });
});

describe("RecordStore threads and DMs", () => {
  const thread: Channel = {
    id: id(40),
    community: aspen.id,
    parentCategory: null,
    name: "",
    sortIndex: 0,
    ty: "thread",
    parentChannel: general.id,
    starterMessage: id(1001),
    replyCount: 1,
    lastReplyAt: "2026-09-25T12:00:00Z",
    recipients: [],
  };
  const dm = (n: number, recipients: string[], ty: "dm" | "groupDm" = "dm"): Channel => ({
    id: id(n),
    community: null,
    parentCategory: null,
    name: "",
    sortIndex: 0,
    ty,
    replyCount: 0,
    recipients,
  });

  it("leaves only reading in a one-to-one DM with someone the caller blocked", () => {
    const store = bootstrapped();
    const withBob = dm(700, [me.id, bob.id]);
    const group = dm(701, [me.id, bob.id, id(3)], "groupDm");
    store.ingest({ channels: [withBob, group] });
    expect(store.channelAccess(withBob.id).has("sendMessages")).toBe(true);
    store.setBlocked(bob.id, true);
    expect(store.blockedDmPeer(withBob.id)).toBe(bob.id);
    expect(Array.from(store.channelAccess(withBob.id))).toEqual(["viewChannel"]);
    // A group stays open to both.
    expect(store.blockedDmPeer(group.id)).toBeNull();
    expect(store.channelAccess(group.id).has("sendMessages")).toBe(true);
    store.setBlocked(bob.id, false);
    expect(store.channelAccess(withBob.id).has("sendMessages")).toBe(true);
  });

  it("leaves only reading in the system account's DM, once its record says what it is", () => {
    const store = bootstrapped();
    const system: User = { ...bob, id: id(9), name: "system", system: true };
    const notices = dm(702, [me.id, system.id]);
    store.ingest({ channels: [notices] });
    // Until the record arrives it is a DM like any other.
    expect(store.channelAccess(notices.id).has("sendMessages")).toBe(true);
    store.ingest({ users: [system] });
    expect(store.systemDmPeer(notices.id)).toBe(system.id);
    expect(Array.from(store.channelAccess(notices.id))).toEqual(["viewChannel"]);
  });

  it("keeps threads out of their community's channel list while holding them as channels", () => {
    const store = bootstrapped();
    store.ingest({ channels: [thread] });
    expect(store.channels(aspen.id).map((c) => c.id)).toEqual([general.id, dev.id]);
    expect(store.channel(thread.id)?.replyCount).toBe(1);
    store.applyEvent({
      serverEvent: "channel",
      type: "update",
      id: thread.id,
      replyCount: 2,
      lastReplyAt: "2026-09-25T13:00:00Z",
    });
    expect(store.channel(thread.id)?.replyCount).toBe(2);
  });

  it("holds the replies echoes show, which no window of the channel includes", () => {
    const store = bootstrapped();
    // A read's record, whose card (none here) is typed by the OpenAPI document.
    const reply = { ...message(7, thread.id), content: "a reply", card: null };
    store.ingest({ messages: [reply] });
    expect(store.message(reply.id)?.content).toBe("a reply");
    expect(store.messages(general.id)).toBeUndefined();
  });

  it("dates a listed DM by its read state until messages arrive in it", () => {
    const store = bootstrapped();
    const talked = dm(61, [me.id, id(3)]);
    store.ingest({
      channels: [talked],
      readStates: [{ channel: talked.id, lastRead: id(900), lastMessage: id(800), mentions: 0 }],
    });
    // The caller's own post moved where they read to, past the newest message by anyone else.
    expect(store.dmActivity(talked.id)).toBe(id(900));
  });

  it("lists DMs in the server's order until activity moves one up", () => {
    const store = bootstrapped();
    const older = dm(50, [me.id, bob.id]);
    const newer = dm(51, [me.id, bob.id, id(3)], "groupDm");
    store.setDms([newer, older]);
    expect(store.dms().map((c) => c.id)).toEqual([newer.id, older.id]);
    expect(store.channels(aspen.id).some((c) => c.id === older.id)).toBe(false);
    store.applyEvent({ serverEvent: "message", type: "create", ...message(9, older.id, bob.id) });
    expect(store.dms().map((c) => c.id)).toEqual([older.id, newer.id]);
    // A DM made after the listing is newer than anything in it.
    const fresh = dm(2000, [me.id, id(3)]);
    store.applyEvent({ serverEvent: "channel", type: "create", ...fresh });
    expect(store.dms()[0]?.id).toBe(fresh.id);
  });

  it("drops a DM the caller left, with its history", () => {
    const store = bootstrapped();
    const group = dm(52, [me.id, bob.id, id(3)], "groupDm");
    store.setDms([group]);
    store.replaceWindow(group.id, [message(3, group.id)], { hasOlder: false, atLatest: true });
    store.applyEvent({
      serverEvent: "channel",
      type: "update",
      id: group.id,
      recipients: [me.id, bob.id],
    });
    expect(store.channel(group.id)?.recipients).toEqual([me.id, bob.id]);
    store.applyEvent({
      serverEvent: "channel",
      type: "update",
      id: group.id,
      recipients: [bob.id],
    });
    expect(store.channel(group.id)).toBeUndefined();
    expect(store.channelRemoved(group.id)).toBe(true);
    expect(store.dms()).toEqual([]);
    expect(store.messages(group.id)).toBeUndefined();
    // Added back, it is held again.
    store.applyEvent({ serverEvent: "channel", type: "create", ...group });
    expect(store.channelRemoved(group.id)).toBe(false);
  });

  it("forgets DMs a new listing no longer has", () => {
    const store = bootstrapped();
    const kept = dm(53, [me.id, bob.id]);
    const gone = dm(54, [me.id, id(3)]);
    store.setDms([kept, gone]);
    store.setDms([kept]);
    expect(store.dms().map((c) => c.id)).toEqual([kept.id]);
    expect(store.channel(gone.id)).toBeUndefined();
  });
});

describe("RecordStore roles and access", () => {
  const everyone = {
    id: id(40),
    community: aspen.id,
    name: "everyone",
    position: 0,
    permissions: [...TEMPLATES.member],
    everyone: true,
  };
  const moderator = {
    id: id(41),
    community: aspen.id,
    name: "Moderator",
    position: 1,
    permissions: ["manageMessages" as const],
    everyone: false,
  };

  function withRoles(): RecordStore {
    const store = bootstrapped();
    store.ingest({ roles: [everyone, moderator], channelOverrides: [], categoryOverrides: [] });
    return store;
  }

  it("resolves the caller's permissions from the roles they hold", () => {
    const store = withRoles();
    expect(store.access(aspen.id)?.has("sendMessages")).toBe(true);
    expect(store.access(aspen.id)?.has("manageMessages")).toBe(false);
    store.applyEvent({
      serverEvent: "userCommunity",
      type: "update",
      community: aspen.id,
      user: me.id,
      roles: [moderator.id],
    });
    expect(store.access(aspen.id)?.has("manageMessages")).toBe(true);
    expect(store.memberRoles(aspen.id, me.id)).toEqual([moderator.id]);
    // Someone else's roles are resolved on request, not for the caller.
    expect(store.access(aspen.id, bob.id)?.has("manageMessages")).toBe(false);
  });

  it("keeps each member's nickname per community, as reads and events give it", () => {
    const store = new RecordStore();
    store.setBootstrap(me, [aspen, birch], {
      users: [me, bob],
      userCommunities: [
        { community: aspen.id, user: bob.id, sortIndex: 0, roles: [], nickname: "Bobbin" },
        { community: birch.id, user: bob.id, sortIndex: 0, roles: [] },
      ],
    });
    expect(store.nickname(aspen.id, bob.id)).toBe("Bobbin");
    expect(store.nickname(birch.id, bob.id)).toBeUndefined();
    expect(store.nicknames(aspen.id).get(bob.id)).toBe("Bobbin");
    const listener = vi.fn();
    store.subscribe(`nicknames:${aspen.id}`, listener);
    // An update without the field leaves the nickname alone.
    store.applyEvent({
      serverEvent: "userCommunity",
      type: "update",
      community: aspen.id,
      user: bob.id,
      roles: [],
    });
    expect(store.nickname(aspen.id, bob.id)).toBe("Bobbin");
    expect(listener).not.toHaveBeenCalled();
    store.applyEvent({
      serverEvent: "userCommunity",
      type: "update",
      community: aspen.id,
      user: bob.id,
      nickname: "Robert",
    });
    expect(store.nickname(aspen.id, bob.id)).toBe("Robert");
    expect(listener).toHaveBeenCalledTimes(1);
    // `null` clears it.
    store.applyEvent({
      serverEvent: "userCommunity",
      type: "update",
      community: aspen.id,
      user: bob.id,
      nickname: null,
    });
    expect(store.nickname(aspen.id, bob.id)).toBeUndefined();
    expect(store.nicknames(aspen.id).size).toBe(0);
    store.noteMemberships([
      { community: aspen.id, user: bob.id, sortIndex: null, roles: [], nickname: "Bob B." },
    ]);
    expect(store.nickname(aspen.id, bob.id)).toBe("Bob B.");
    // Leaving takes it with the membership.
    store.applyEvent({
      serverEvent: "userCommunity",
      type: "delete",
      community: aspen.id,
      user: bob.id,
    });
    expect(store.nickname(aspen.id, bob.id)).toBeUndefined();
  });

  it("lets go of a channel the caller may no longer view, and keeps it for its role", () => {
    const store = withRoles();
    const listener = vi.fn();
    store.subscribe(`channelAccess:${dev.id}`, listener);
    store.applyEvent({
      serverEvent: "categoryOverride",
      type: "create",
      category: work.id,
      role: everyone.id,
      allow: [],
      deny: ["sendMessages"],
    });
    expect(listener).toHaveBeenCalled();
    expect(store.channelAccess(dev.id).has("sendMessages")).toBe(false);
    expect(store.channelAccess(general.id).has("sendMessages")).toBe(true);
    store.applyEvent({
      serverEvent: "channelOverride",
      type: "create",
      channel: general.id,
      role: everyone.id,
      allow: [],
      deny: ["viewChannel"],
    });
    expect(store.channel(general.id)).toBeUndefined();
    expect(store.channelRemoved(general.id)).toBe(true);
    expect(store.channel(dev.id)).toBeDefined();
  });

  it("lets go of a channel moved where the caller may not view it, and of its pins", () => {
    const store = withRoles();
    store.setPins(dev.id, []);
    store.applyEvent({
      serverEvent: "categoryOverride",
      type: "create",
      category: work.id,
      role: everyone.id,
      allow: [],
      deny: ["viewChannel"],
    });
    expect(store.channel(dev.id)).toBeUndefined();
    expect(store.pins(dev.id)).toBeUndefined();
    // Moving general into the hidden category is the last the caller hears of it.
    store.applyEvent({
      serverEvent: "channel",
      type: "update",
      id: general.id,
      parentCategory: work.id,
    });
    expect(store.channel(general.id)).toBeUndefined();
  });

  it("forgets the bans it held once the caller may not ban", () => {
    const store = withRoles();
    store.applyEvent({
      serverEvent: "userCommunity",
      type: "update",
      community: aspen.id,
      user: me.id,
      roles: [moderator.id],
    });
    store.applyEvent({
      serverEvent: "role",
      type: "update",
      id: moderator.id,
      permissions: ["banMembers"],
    });
    store.replaceBans(aspen.id, []);
    expect(store.bans(aspen.id)).toEqual([]);
    store.applyEvent({
      serverEvent: "role",
      type: "update",
      id: moderator.id,
      permissions: [],
    });
    expect(store.bans(aspen.id)).toBeUndefined();
  });

  it("gives the owner everything and forgets a deleted role's overrides", () => {
    const store = withRoles();
    store.applyEvent({
      serverEvent: "channelOverride",
      type: "create",
      channel: dev.id,
      role: moderator.id,
      allow: ["viewChannel"],
      deny: [],
    });
    expect(store.channelOverrides(dev.id)).toHaveLength(1);
    store.applyEvent({ serverEvent: "role", type: "delete", id: moderator.id });
    expect(store.channelOverrides(dev.id)).toHaveLength(0);
    expect(store.roles(aspen.id).map((r) => r.id)).toEqual([everyone.id]);
    store.applyEvent({ serverEvent: "community", type: "update", id: aspen.id, owner: me.id });
    expect(store.access(aspen.id)?.owner).toBe(true);
    expect(store.access(aspen.id)?.has("manageRoles")).toBe(true);
  });
});

describe("RecordStore deployment moderation", () => {
  const everyone = {
    id: id(40),
    community: aspen.id,
    name: "everyone",
    position: 0,
    permissions: [...TEMPLATES.member],
    everyone: true,
  };

  it("lets a moderator see a hidden channel and take messages down, by their own event", () => {
    const store = bootstrapped();
    store.ingest({
      roles: [everyone],
      channelOverrides: [
        { channel: general.id, role: everyone.id, allow: [], deny: ["viewChannel"] },
      ],
      categoryOverrides: [],
    });
    expect(store.channel(general.id)).toBeUndefined();
    store.ingest({ channels: [general] });
    store.applyEvent({
      serverEvent: "deploymentAccessChanged",
      permissions: ["viewDashboard", "moderateCommunities"],
    });
    expect(store.moderator).toBe(true);
    expect(store.channelAccess(general.id).has("viewChannel")).toBe(true);
    expect(store.access(aspen.id)?.has("manageMessages")).toBe(true);
    expect(store.access(aspen.id)?.moderating("manageMessages")).toBe(true);
    expect(store.access(aspen.id)?.moderating("sendMessages")).toBe(false);
    // Losing it lets the hidden channel go again.
    store.applyEvent({ serverEvent: "deploymentAccessChanged", permissions: [] });
    expect(store.channel(general.id)).toBeUndefined();
  });
});

describe("RecordStore invites and Manage invites", () => {
  it("lets go of other people's invites once the caller may not manage them", () => {
    const store = bootstrapped();
    const everyone = {
      id: id(40),
      community: aspen.id,
      name: "everyone",
      position: 0,
      permissions: [...TEMPLATES.member, "manageInvites" as const],
      everyone: true,
    };
    store.ingest({ roles: [everyone], channelOverrides: [], categoryOverrides: [] });
    const invite = (code: string, createdBy: string) => ({
      code,
      community: aspen.id,
      createdBy,
      createdAt: "2026-09-28T00:00:00Z",
      expiresAt: null,
    });
    store.applyEvent({ serverEvent: "invite", type: "create", ...invite("MINE", me.id) });
    store.applyEvent({ serverEvent: "invite", type: "create", ...invite("BOBS", bob.id) });
    expect(store.invite("BOBS")).toBeDefined();
    store.applyEvent({
      serverEvent: "role",
      type: "update",
      id: everyone.id,
      permissions: [...TEMPLATES.member],
    });
    expect(store.invite("BOBS")).toBeUndefined();
    expect(store.invite("MINE")).toBeDefined();
  });
});

describe("RecordStore blocks", () => {
  it("follows blocks from events and from a bootstrap's list", () => {
    const store = bootstrapped();
    expect(store.blocked(bob.id)).toBe(false);
    store.applyEvent({ serverEvent: "userBlockChanged", user: bob.id, blocked: true });
    expect(store.blocked(bob.id)).toBe(true);
    expect(store.blockedUsers()).toEqual([bob.id]);
    expect(store.setBlocked(bob.id, true)).toBe(false);
    store.replaceBlocks([]);
    expect(store.blocked(bob.id)).toBe(false);
    expect(store.blockedUsers()).toEqual([]);
  });

  it("silences someone blocked on another deployment, once it knows who they are", () => {
    const store = bootstrapped();
    const heard = vi.fn();
    store.subscribe("silenced", heard);
    // Bob is native here; a guest from a.example is known only once their record arrives.
    store.setBlockedIdentities("b.example", new Set([`b.example/${bob.id}`, "a.example/h1"]));
    expect(heard).toHaveBeenCalledTimes(1);
    expect(store.silenced(bob.id)).toBe(true);
    expect(store.blocked(bob.id)).toBe(false);
    const guest = { ...bob, id: id(3), name: "guest", homeDomain: "a.example", homeId: "h1" };
    expect(store.silenced(guest.id)).toBe(false);
    store.ingest({ users: [guest] });
    expect(heard).toHaveBeenCalledTimes(2);
    expect(store.silenced(guest.id)).toBe(true);
    store.setBlockedIdentities("b.example", new Set([`b.example/${bob.id}`, "a.example/h1"]));
    expect(heard).toHaveBeenCalledTimes(2);
    store.setBlockedIdentities("b.example", new Set());
    expect(store.silenced(bob.id)).toBe(false);
    store.setBlocked(bob.id, true);
    expect(store.silenced(bob.id)).toBe(true);
  });

  it("never lets a blocked user's message make a channel unread", () => {
    const store = bootstrapped();
    store.ingest({
      readStates: [{ channel: general.id, lastRead: id(1001), lastMessage: null, mentions: 0 }],
    });
    store.setBlocked(bob.id, true);
    store.applyEvent({ serverEvent: "message", type: "create", ...message(2, general.id, bob.id) });
    expect(store.unread(general.id)).toBe(false);
    store.setBlocked(bob.id, false);
    store.applyEvent({ serverEvent: "message", type: "create", ...message(3, general.id, bob.id) });
    expect(store.unread(general.id)).toBe(true);
  });

  it("leaves a blocked user's reactions out", () => {
    const store = bootstrapped();
    const target = message(1).id;
    store.setBlocked(bob.id, true);
    store.applyEvent({
      serverEvent: "react",
      type: "create",
      messageId: target,
      emoji: "😁",
      userId: bob.id,
    });
    expect(store.reactions(target).size).toBe(0);
  });

  it("keeps a read position further on than the one the server sent", () => {
    const store = bootstrapped();
    store.ingest({
      readStates: [{ channel: general.id, lastRead: id(1005), lastMessage: null, mentions: 0 }],
    });
    store.putReadState({
      channel: general.id,
      lastRead: id(1003),
      lastMessage: id(1006),
      mentions: 0,
    });
    expect(store.readState(general.id)).toEqual({
      channel: general.id,
      lastRead: id(1005),
      lastMessage: id(1006),
      mentions: 0,
    });
  });
});

describe("RecordStore unread tags", () => {
  const tagged = (n: number, mentions: Message["mentions"]): Message => ({
    ...message(n, general.id, bob.id),
    mentions,
  });
  const none = { users: [], roles: [], everyone: false };

  it("counts the unread messages that tag the caller, by name, role, or as everyone", () => {
    const store = bootstrapped();
    store.ingest({
      readStates: [{ channel: general.id, lastRead: id(1001), lastMessage: null, mentions: 0 }],
    });
    store.applyEvent({
      serverEvent: "userCommunity",
      type: "update",
      community: aspen.id,
      user: me.id,
      roles: [id(40)],
    });
    const post = (m: Message) => {
      store.applyEvent({ serverEvent: "message", type: "create", ...m });
    };
    post(tagged(2, { ...none, users: [me.id] }));
    post(tagged(3, { ...none, roles: [id(40)] }));
    post(tagged(4, { ...none, everyone: true }));
    post(tagged(5, { ...none, users: [bob.id] }));
    post(tagged(6, none));
    expect(store.mentions(general.id)).toBe(3);
    expect(store.placeMentions(aspen.id)).toBe(3);
    // Read to the newest, nothing tags them any more.
    store.setLastRead(general.id, id(1006));
    expect(store.mentions(general.id)).toBe(0);
  });

  it("clears the tags when the caller posts, and never counts someone blocked", () => {
    const store = bootstrapped();
    store.ingest({
      readStates: [{ channel: general.id, lastRead: id(1001), lastMessage: null, mentions: 0 }],
    });
    store.applyEvent({
      serverEvent: "message",
      type: "create",
      ...tagged(2, { ...none, users: [me.id] }),
    });
    expect(store.mentions(general.id)).toBe(1);
    store.applyEvent({ serverEvent: "message", type: "create", ...message(3) });
    expect(store.mentions(general.id)).toBe(0);
    store.setBlocked(bob.id, true);
    store.applyEvent({
      serverEvent: "message",
      type: "create",
      ...tagged(4, { ...none, everyone: true }),
    });
    expect(store.mentions(general.id)).toBe(0);
  });
});

describe("RecordStore notifications", () => {
  it("resolves a channel's level from its own setting, its community's, and the default", () => {
    const store = bootstrapped();
    expect(store.notificationLevel(general.id)).toEqual({
      level: "tags",
      own: null,
      inherited: "tags",
    });
    store.replaceNotificationSettings([{ community: aspen.id, channel: null, level: "all" }]);
    expect(store.notificationLevel(general.id).level).toBe("all");
    store.applyEvent({
      serverEvent: "notificationSettingChanged",
      community: null,
      channel: general.id,
      level: "nothing",
    });
    expect(store.notificationLevel(general.id)).toEqual({
      level: "nothing",
      own: "nothing",
      inherited: "all",
    });
    expect(store.notificationLevel(dev.id).level).toBe("all");
    store.applyEvent({
      serverEvent: "notificationSettingChanged",
      community: null,
      channel: general.id,
      level: null,
    });
    expect(store.notificationLevel(general.id).level).toBe("all");
    expect(store.communityNotificationLevel(aspen.id)).toBe("all");
  });

  it("notifies of others' unread messages as the level asks, never muted or blocked ones", () => {
    const store = bootstrapped();
    store.ingest({
      readStates: [{ channel: general.id, lastRead: id(1000), lastMessage: null, mentions: 0 }],
    });
    const plain = message(2, general.id, bob.id);
    const tagging = {
      ...message(3, general.id, bob.id),
      mentions: { users: [me.id], roles: [], everyone: false },
    };
    expect(store.notifies(plain)).toBe(false);
    expect(store.notifies(tagging)).toBe(true);
    expect(store.notifies({ ...tagging, author: me.id })).toBe(false);
    expect(store.notifies({ ...tagging, id: id(999) })).toBe(false);
    store.replaceNotificationSettings([{ community: null, channel: general.id, level: "all" }]);
    expect(store.notifies(plain)).toBe(true);
    store.replaceMutes([{ channel: general.id, until: null }]);
    expect(store.notifies(plain)).toBe(false);
    store.replaceMutes([]);
    store.setBlocked(bob.id, true);
    expect(store.notifies(plain)).toBe(false);
  });
});

describe("RecordStore plugins", () => {
  const filter = {
    id: "org.example.filter",
    version: "1.0.0",
    name: "Filter",
    description: "Filters",
    mode: "optIn" as const,
    dms: false,
    principal: null,
    principalPermissions: [],
    communitySettings: [],
    channelTypes: [],
    messages: { watched: "Watched" },
  };

  function note(n: number, messageId: string, plugin = filter.id) {
    return {
      id: id(5000 + n),
      message: messageId,
      plugin,
      kind: `kind${String(n)}`,
      severity: "notice" as const,
      label: { key: "watched" },
    };
  }

  it("holds a message's annotations as reads list them, and only of plugins it runs", () => {
    const store = bootstrapped();
    store.setPlugins([filter]);
    const first = message(1);
    store.replaceWindow(general.id, [first], { hasOlder: false, atLatest: true });
    store.setAnnotations([first.id], [note(1, first.id), note(2, first.id, "org.example.gone")]);
    expect(store.annotations(first.id).map((a) => a.kind)).toEqual(["kind1"]);
    // A read that lists none for the message leaves it with none.
    store.setAnnotations([first.id], []);
    expect(store.annotations(first.id)).toEqual([]);
  });

  it("applies annotation events, finding an update's message by the annotation", () => {
    const store = bootstrapped();
    store.setPlugins([filter]);
    const first = message(1);
    store.replaceWindow(general.id, [first], { hasOlder: false, atLatest: true });
    const listener = vi.fn();
    store.subscribe(`annotations:${first.id}`, listener);
    store.applyEvent({ serverEvent: "messageAnnotation", type: "create", ...note(1, first.id) });
    expect(store.annotations(first.id)).toHaveLength(1);
    store.applyEvent({
      serverEvent: "messageAnnotation",
      type: "update",
      id: id(5001),
      severity: "warning",
    });
    expect(store.annotations(first.id)[0]?.severity).toBe("warning");
    store.applyEvent({ serverEvent: "messageAnnotation", type: "delete", id: id(5001) });
    expect(store.annotations(first.id)).toEqual([]);
    expect(listener).toHaveBeenCalledTimes(3);
  });

  it("drops a message's annotations with the message", () => {
    const store = bootstrapped();
    store.setPlugins([filter]);
    const first = message(1);
    store.replaceWindow(general.id, [first], { hasOlder: false, atLatest: true });
    store.setAnnotations([first.id], [note(1, first.id)]);
    store.applyEvent({ serverEvent: "message", type: "delete", id: first.id });
    expect(store.annotations(first.id)).toEqual([]);
  });

  it("hides a plugin's annotations once the catalogue no longer lists it", () => {
    const store = bootstrapped();
    store.setPlugins([filter]);
    const first = message(1);
    store.replaceWindow(general.id, [first], { hasOlder: false, atLatest: true });
    store.setAnnotations([first.id], [note(1, first.id)]);
    expect(store.annotations(first.id)).toHaveLength(1);
    store.setPlugins([]);
    expect(store.annotations(first.id)).toEqual([]);
  });

  it("keeps a community's plugin settings current for its managers", () => {
    const store = bootstrapped();
    expect(store.communityPlugins(aspen.id)).toBeUndefined();
    store.setCommunityPlugins(aspen.id, [
      { community: aspen.id, plugin: filter.id, enabled: false, settings: {}, secretsSet: [] },
    ]);
    store.applyEvent({
      serverEvent: "communityPlugin",
      type: "update",
      community: aspen.id,
      plugin: filter.id,
      enabled: true,
    });
    expect(store.communityPlugins(aspen.id)?.[0]?.enabled).toBe(true);
  });
});

describe("RecordStore previews and held messages", () => {
  const photo = {
    id: id(6000),
    fileName: "photo.jpg",
    mimeType: "image/jpeg",
    downloadUrl: "https://media.example/attachments/photo",
    width: 4032,
    height: 3024,
  };
  const preview = {
    url: "https://media.example/attachment-previews/photo",
    mimeType: "image/webp",
    width: 1280,
    height: 960,
  };
  const held = {
    id: id(6100),
    channelId: general.id,
    content: "look",
    attachments: [photo.id],
    echoToParent: false,
    heldAt: "2026-10-05T12:00:00Z",
  };

  it("gives an attachment the preview made after it was read", () => {
    const store = new RecordStore();
    store.ingest({ attachments: [photo] });
    store.applyEvent({
      serverEvent: "attachmentPreviewed",
      attachment: photo.id,
      message: null,
      preview,
    });
    expect(store.attachment(photo.id)?.preview).toEqual(preview);
  });

  it("keeps a preview when a read from before it was made lands after its event", () => {
    const store = new RecordStore();
    store.ingest({ attachments: [{ ...photo, preview }] });
    store.ingest({ attachments: [photo] });
    expect(store.attachment(photo.id)?.preview).toEqual(preview);
  });

  it("shows a held message until it is posted", () => {
    const store = new RecordStore();
    store.putHeldMessage(held);
    expect(store.heldMessages(general.id)).toEqual([{ message: held, failure: null }]);
    expect(store.heldMessages(dev.id)).toEqual([]);
    store.applyEvent({
      serverEvent: "heldMessagePosted",
      held: held.id,
      channel: general.id,
      message: id(6200),
    });
    expect(store.heldMessages(general.id)).toEqual([]);
  });

  it("does not bring back a held message whose posting was heard of before its 202", () => {
    const store = new RecordStore();
    store.applyEvent({
      serverEvent: "heldMessagePosted",
      held: held.id,
      channel: general.id,
      message: id(6200),
    });
    store.putHeldMessage(held);
    expect(store.heldMessages(general.id)).toEqual([]);
  });

  it("keeps a dropped held message, with why, through a fresh read, until it is let go", () => {
    const store = new RecordStore();
    store.putHeldMessage(held);
    store.applyEvent({
      serverEvent: "heldMessageFailed",
      held: held.id,
      channel: general.id,
      detail: "You can no longer post here.",
    });
    store.replaceHeldMessages([]);
    expect(store.heldMessages(general.id)).toEqual([
      { message: held, failure: "You can no longer post here." },
    ]);
    store.forgetHeldMessage(held.id);
    expect(store.heldMessages(general.id)).toEqual([]);
  });
});
