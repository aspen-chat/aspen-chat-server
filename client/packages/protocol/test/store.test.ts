import { describe, expect, it, vi } from "vitest";
import {
  RecordStore,
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

const me: User = { id: id(1), name: "kate", icon: null, onlineStatus: "online" };
const bob: User = { id: id(2), name: "bob", icon: null, onlineStatus: "offline" };
const aspen: Community = { id: id(10), name: "Aspen", icon: null };
const birch: Community = { id: id(11), name: "Birch", icon: null };
const general: Channel = {
  id: id(20),
  community: aspen.id,
  parentCategory: null,
  name: "general",
  sortIndex: 0,
  ty: "Text",
};
const dev: Channel = {
  id: id(21),
  community: aspen.id,
  parentCategory: id(30),
  name: "dev",
  sortIndex: 1,
  ty: "Text",
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
  };
}

function bootstrapped(): RecordStore {
  const store = new RecordStore();
  store.setBootstrap(me, [aspen, birch], {
    channels: [general, dev],
    categories: [work],
    users: [me, bob],
    userCommunities: [
      { community: aspen.id, user: me.id, sortIndex: 0 },
      { community: aspen.id, user: bob.id, sortIndex: 0 },
      { community: birch.id, user: me.id, sortIndex: 0 },
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
      userCommunities: [{ community: aspen.id, user: me.id, sortIndex: 0 }],
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
    });
    expect(store.communities()).toEqual([aspen, birch, cedar]);
    store.applyEvent({
      serverEvent: "userCommunity",
      type: "delete",
      community: aspen.id,
      user: me.id,
    });
    expect(store.communities()).toEqual([birch, cedar]);
    expect(store.memberIds(aspen.id)).toEqual([bob.id]);
  });

  it("regroups member records when a member's status changes", () => {
    const store = bootstrapped();
    const onMembers = vi.fn();
    store.subscribe(`members:${aspen.id}`, onMembers);
    const before = store.members(aspen.id);
    expect(before.map((u) => u.name)).toEqual(["kate", "bob"]);
    store.applyEvent({ serverEvent: "userStatus", id: bob.id, status: "online" });
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
    store.applyEvent({ serverEvent: "userStatus", id: bob.id, status: "online" });
    expect(store.user(bob.id)?.onlineStatus).toBe("online");
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
    expect(Array.from(first.get("😁") ?? [])).toEqual([me.id, bob.id]);
    store.applyEvent({
      serverEvent: "react",
      type: "delete",
      messageId: target,
      emoji: "😁",
      userId: me.id,
    });
    expect(store.reactions(target)).not.toBe(first);
    expect(Array.from(store.reactions(target).get("😁") ?? [])).toEqual([bob.id]);
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

  it("applies invite events, but only creates for the caller's own communities", () => {
    const store = bootstrapped();
    store.applyEvent({ serverEvent: "invite", type: "create", ...invite("mine") });
    store.applyEvent({ serverEvent: "invite", type: "create", ...invite("theirs", id(99)) });
    expect(store.invite("mine")).toBeDefined();
    expect(store.invite("theirs")).toBeUndefined();
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
    store.replaceWindow(general.id, page(200, 100), { hasOlder: true, atLatest: true });
    store.prependWindow(general.id, page(100, 100), true);
    const window = store.messages(general.id);
    expect(window?.ids).toHaveLength(WINDOW_MAX_MESSAGES);
    expect(window?.ids[0]).toBe(message(100).id);
    expect(window?.ids.at(-1)).toBe(message(249).id);
    expect(window?.atLatest).toBe(false);
    expect(store.message(message(299).id)).toBeUndefined();
    expect(store.message(message(249).id)).toBeDefined();
  });

  it("drops the oldest messages when newer ones push the window past its cap", () => {
    const store = bootstrapped();
    store.replaceWindow(general.id, page(100, 100), { hasOlder: false, atLatest: false });
    store.appendWindow(general.id, page(200, 100), true);
    const window = store.messages(general.id);
    expect(window?.ids).toHaveLength(WINDOW_MAX_MESSAGES);
    expect(window?.ids[0]).toBe(message(150).id);
    expect(window?.ids.at(-1)).toBe(message(299).id);
    expect(window).toMatchObject({ hasOlder: true, atLatest: true });
    expect(store.message(message(100).id)).toBeUndefined();
  });

  it("keeps a live window bounded as messages arrive", () => {
    const store = bootstrapped();
    store.replaceWindow(general.id, page(100, WINDOW_MAX_MESSAGES), {
      hasOlder: false,
      atLatest: true,
    });
    store.applyEvent({ serverEvent: "message", type: "create", ...message(900) });
    const window = store.messages(general.id);
    expect(window?.ids).toHaveLength(WINDOW_MAX_MESSAGES);
    expect(window?.ids[0]).toBe(message(101).id);
    expect(window?.hasOlder).toBe(true);
    expect(store.message(message(100).id)).toBeUndefined();
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
      userCommunities: [{ community: aspen.id, user: me.id, sortIndex: 0 }],
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
    expect(store.channelVoice(general.id)).toEqual({ session: null, participants: [] });
    expect(listener).toHaveBeenCalled();
  });

  it("drops a call with its channel", () => {
    const store = bootstrapped();
    store.applyEvent({ serverEvent: "voiceSession", type: "create", ...session });
    store.applyEvent({ serverEvent: "channel", type: "delete", id: general.id });
    expect(store.channelVoice(general.id).session).toBeNull();
  });
});
