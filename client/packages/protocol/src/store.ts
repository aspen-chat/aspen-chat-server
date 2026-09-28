/**
 * The client's cache of server records, normalized by type and id.
 *
 * Every record the UI renders comes from here. REST responses are ingested into it and server
 * events are applied to it as JSON Merge Patches, so a record has one representation whether it
 * arrived by bootstrap, by a sideloaded read, or by the event stream. Nothing in this file does
 * I/O: `AspenSync` decides what to fetch and when, and this store only holds the results.
 *
 * Reads are organised around topics (see `Topic`). Each getter is memoized under one topic and
 * returns the same reference until a write touches that topic, which is what React's
 * `useSyncExternalStore` needs to skip re-renders. Writes are batched: a single event may touch
 * several topics, and every listener is notified once, after the whole write has been applied.
 */

import {
  DM_PERMISSIONS,
  resolveCommunity,
  type CommunityPermissions,
  type PermissionSet,
} from "./permissions";
import type {
  Category,
  CategoryOverride,
  Channel,
  ChannelOverride,
  Community,
  Invite,
  Message,
  Poll,
  Role,
  ServerEvent,
  VoiceParticipant,
  VoiceSession,
  User,
  UserCommunity,
} from "./generated/events";
import type { components } from "./generated/openapi";

type UserOnlineStatus = components["schemas"]["UserOnlineStatus"];

export type Attachment = components["schemas"]["Attachment"];
export type Icon = components["schemas"]["Icon"];
export type Included = components["schemas"]["Included"];
export type PollVote = components["schemas"]["PollVote"];
/**
 * How far the caller has read a channel. `lastRead` is a position among the channel's message
 * ids, whose lexical order is chronological, so the channel is unread while `lastMessage` sorts
 * after it. An empty `lastRead` is before every message.
 */
export type ReadState = components["schemas"]["ReadState"];
/** A channel the caller has muted; `until` is `null` for a mute that lasts until lifted. */
export type ChannelMute = components["schemas"]["ChannelMute"];
/** A pinned message: which, when it was pinned, and its place among the channel's pins. */
export type Pin = components["schemas"]["Pin"];

export type Listener = () => void;

/**
 * A subscription key. Topics are strings so that one map holds every listener, and so a topic
 * can be computed from an id without allocating an object per subscription.
 *
 * - `me`, `communities`: the calling user and the communities they belong to
 * - `community:<id>`, `channel:<id>`, `category:<id>`, `user:<id>`, `message:<id>`,
 *   `attachment:<id>`: one record
 * - `channels:<communityId>`, `categories:<communityId>`, `members:<communityId>`: a
 *   community's children, as ids or records
 * - `messages:<channelId>`: the loaded window of a channel's history (a thread's too)
 * - `dms`: the caller's DMs and group DMs, the most recently active first
 * - `people`: everyone the caller shares a community with, as far as the members read so far
 *   show, which is who they may start a DM with
 * - `reactions:<messageId>`: who reacted with what
 * - `poll:<id>`: one poll with its tally, and the calling user's own votes on it
 * - `icon:<id>`: one uploaded icon, which users and communities name by id
 * - `voice:<channelId>`: the call on a voice channel and who is in it
 * - `invites:<communityId>`, `invite:<code>`: a community's invites, once loaded
 * - `roles:<communityId>`: a community's roles, lowest first, and which of them each member
 *   holds
 * - `overrides:<channelId|categoryId>`: one channel's or category's overrides
 * - `access:<communityId>`: what the caller may do across a community
 * - `channelAccess:<channelId>`: what the caller may do in one channel
 * - `pins:<channelId>`: a channel's pinned messages, once loaded
 */
export type Topic = string;

/**
 * The most messages a channel's window holds. Extending it past this at one end drops
 * messages, and their records, from the other end, so a long scroll through history never
 * holds more than this many messages in memory; what was dropped is read again on the way back.
 */
export const WINDOW_MAX_MESSAGES = 150;

/** Stands for the DMs among `RecordStore.unreadPlaces`, beside community ids. */
export const UNREAD_DMS = "dms";

/**
 * The loaded portion of a channel's history. Message ids are UUIDv7, so their lexical order is
 * chronological and `ids` is kept sorted ascending. `atLatest` means the newest id is the
 * channel's newest message, so messages created from now on belong at the end; a window loaded
 * around an old message, or trimmed at its newer end, is not at the latest, and new messages
 * are not appended to it because they would not be contiguous with it.
 */
export interface MessageWindow {
  readonly ids: readonly string[];
  readonly hasOlder: boolean;
  readonly atLatest: boolean;
}

/** Someone in a call, with what the client tracks about them from speaking events. */
export interface VoiceParticipantState {
  readonly user: string;
  readonly session: string;
  readonly channel: string;
  readonly joinedAt: string;
  readonly muted: boolean;
  readonly sharingScreen: boolean;
  readonly deafened: boolean;
  readonly speaking: boolean;
  /** When they last started speaking, as `now()` reported it; `null` if never seen speaking. */
  readonly lastSpokeAt: number | null;
}

/** A voice channel's call, or none. */
export interface ChannelVoice {
  readonly session: VoiceSession | null;
  /** In the order they joined. */
  readonly participants: readonly VoiceParticipantState[];
}

const NO_VOICE: ChannelVoice = { session: null, participants: [] };

/**
 * One emoji's reactions to a message, in brief: how many, whether the caller is among them, and
 * the first few to react, earliest first (`REACTION_SUMMARY_USERS` at most).
 */
export interface EmojiReactions {
  readonly count: number;
  readonly me: boolean;
  readonly users: readonly string[];
}

/** A message's reactions, `emoji -> summary`, each emoji in the order it was first used. */
export type Reactions = ReadonlyMap<string, EmojiReactions>;

/** A message read's summary of one emoji's reactions. */
export type ReactionSummary = components["schemas"]["ReactionSummary"];

/** How many of an emoji's reactors a summary names, as the server's `SUMMARY_USERS`. */
export const REACTION_SUMMARY_USERS = 4;

const EMPTY_IDS: readonly string[] = [];
const NO_PERMISSIONS: PermissionSet = new Set();
const EMPTY_OVERRIDES: readonly never[] = [];
const EMPTY_REACTIONS: Reactions = new Map();
const EMPTY_VOTES: ReadonlySet<number> = new Set();

function compareBySortIndex<T extends { sortIndex: number; name: string }>(a: T, b: T): number {
  return a.sortIndex - b.sortIndex || a.name.localeCompare(b.name);
}

/** Index at which `id` is, or would be inserted, in an ascending list of ids. */
function lowerBound(ids: readonly string[], id: string): number {
  let low = 0;
  let high = ids.length;
  while (low < high) {
    const mid = (low + high) >>> 1;
    const candidate = ids[mid];
    if (candidate !== undefined && candidate < id) {
      low = mid + 1;
    } else {
      high = mid;
    }
  }
  return low;
}

/**
 * Applies a JSON Merge Patch: every field present in `patch` is written, including `null`,
 * which clears a nullable field; absent fields are left alone. `type` and `serverEvent` are the
 * event's own tags and never part of the record.
 */
function mergePatch<T extends object>(record: T, patch: object): T {
  const next: Record<string, unknown> = { ...(record as Record<string, unknown>) };
  for (const [key, value] of Object.entries(patch)) {
    if (key === "type" || key === "serverEvent" || value === undefined) {
      continue;
    }
    next[key] = value;
  }
  return next as T;
}

/** The record carried by a `create` event, without the event's own tags. */
function created<T extends object>(event: T & { type: "create"; serverEvent: string }): T {
  const record: Record<string, unknown> = { ...(event as Record<string, unknown>) };
  delete record.type;
  delete record.serverEvent;
  return record as T;
}

export class RecordStore {
  readonly #now: () => number;
  readonly #users = new Map<string, User>();
  readonly #communities = new Map<string, Community>();
  readonly #channels = new Map<string, Channel>();
  readonly #categories = new Map<string, Category>();
  readonly #messages = new Map<string, Message>();
  readonly #attachments = new Map<string, Attachment>();
  readonly #icons = new Map<string, Icon>();
  readonly #voiceSessions = new Map<string, VoiceSession>();
  /** `session -> user -> participant`. */
  readonly #voiceParticipants = new Map<string, Map<string, VoiceParticipantState>>();
  /** `community -> user ids`, the sample of members the server returns per community. */
  readonly #members = new Map<string, Set<string>>();
  /** `user -> community ids`, the reverse of `#members`, for invalidating member lists. */
  readonly #memberOf = new Map<string, Set<string>>();
  readonly #reactions = new Map<string, Reactions>();
  readonly #polls = new Map<string, Poll>();
  /** `poll -> option indices` the calling user voted for, for polls read with their votes. */
  readonly #myVotes = new Map<string, ReadonlySet<number>>();
  /** The options the caller wrote in, by poll, which an anonymous poll's record never says. */
  readonly #myWriteIns = new Map<string, ReadonlySet<number>>();
  readonly #readStates = new Map<string, ReadState>();
  readonly #mutes = new Map<string, ChannelMute>();
  readonly #collapsed = new Set<string>();
  readonly #roles = new Map<string, Role>();
  /** Overrides by `channel/role` and `category/role`. */
  readonly #channelOverrides = new Map<string, ChannelOverride>();
  readonly #categoryOverrides = new Map<string, CategoryOverride>();
  /** `channel -> message -> pin`, for the channels whose pins have been loaded. */
  readonly #pins = new Map<string, Map<string, Pin>>();
  /** `community/user -> role ids` each member holds besides everyone's, as far as known. */
  readonly #memberRoles = new Map<string, readonly string[]>();
  #admin = false;
  readonly #windows = new Map<string, MessageWindow>();
  /** Invites by code, for the communities whose invite lists have been loaded. */
  readonly #invites = new Map<string, Invite>();
  #myUserId: string | null = null;
  /**
   * Communities the calling user belongs to, tracked separately from `#members` because the
   * member sample is capped and need not contain the caller.
   */
  readonly #myCommunities = new Set<string>();
  /** `community -> sortIndex` from the caller's own memberships: the order of their list. */
  readonly #myOrder = new Map<string, number>();
  /** The caller's DMs in the order the server listed them, most recently active first. */
  #dmOrder: string[] = [];
  /**
   * The newest activity seen in each DM since it was listed: the id of its latest message, or
   * the DM's own id when it was made. Both are UUIDv7, so they compare by time, and anything
   * seen live is newer than the listing.
   */
  readonly #dmActivity = new Map<string, string>();
  /**
   * Channels that were held and then removed (deleted, or a DM the caller left), so a screen
   * showing one can tell it is gone rather than not yet read.
   */
  readonly #removedChannels = new Set<string>();

  readonly #listeners = new Map<Topic, Set<Listener>>();
  readonly #memo = new Map<Topic, unknown>();
  readonly #dirty = new Set<Topic>();
  #batchDepth = 0;

  /** `now` stamps when participants speak; tests pass a fake clock. */
  constructor(options: { now?: () => number } = {}) {
    this.#now = options.now ?? (() => Date.now());
  }

  // ---------------------------------------------------------------------------------------
  // Subscriptions

  /** Registers for changes to one topic and returns the unsubscribe function. */
  readonly subscribe = (topic: Topic, listener: Listener): (() => void) => {
    let set = this.#listeners.get(topic);
    if (set === undefined) {
      set = new Set();
      this.#listeners.set(topic, set);
    }
    set.add(listener);
    return () => {
      set.delete(listener);
      if (set.size === 0) {
        this.#listeners.delete(topic);
      }
    };
  };

  // ---------------------------------------------------------------------------------------
  // Reads

  get myUserId(): string | null {
    return this.#myUserId;
  }

  /** Topic `me`. */
  me(): User | null {
    return this.#myUserId === null ? null : (this.#users.get(this.#myUserId) ?? null);
  }

  user(id: string): User | undefined {
    return this.#users.get(id);
  }

  community(id: string): Community | undefined {
    return this.#communities.get(id);
  }

  /** Topic `channel:<id>`: whether the channel was held and has since been removed. */
  channelRemoved(id: string): boolean {
    return this.#removedChannels.has(id);
  }

  channel(id: string): Channel | undefined {
    return this.#channels.get(id);
  }

  category(id: string): Category | undefined {
    return this.#categories.get(id);
  }

  message(id: string): Message | undefined {
    return this.#messages.get(id);
  }

  attachment(id: string): Attachment | undefined {
    return this.#attachments.get(id);
  }

  /** Topic `voice:<channelId>`: the call on the channel and who is in it. */
  channelVoice(channelId: string): ChannelVoice {
    return this.#memoized(`voice:${channelId}`, () => {
      const session = Array.from(this.#voiceSessions.values()).find((s) => s.channel === channelId);
      if (session === undefined) {
        return NO_VOICE;
      }
      const participants = Array.from(this.#voiceParticipants.get(session.id)?.values() ?? []).sort(
        (a, b) => a.joinedAt.localeCompare(b.joinedAt),
      );
      return { session, participants };
    });
  }

  /** Topic `roles:<communityId>`: the community's roles, lowest first. */
  roles(communityId: string): readonly Role[] {
    return this.#memoized(`roles:${communityId}`, () =>
      Array.from(this.#roles.values())
        .filter((r) => r.community === communityId)
        .sort((a, b) => a.position - b.position),
    );
  }

  /**
   * Topic `roles:<communityId>`: the roles a member holds besides everyone's, or `undefined`
   * when no read has said.
   */
  memberRoles(communityId: string, userId: string): readonly string[] | undefined {
    return this.#memberRoles.get(`${communityId}/${userId}`);
  }

  /** Topic `overrides:<channelId>`. */
  channelOverrides(channelId: string): readonly ChannelOverride[] {
    return this.#memoized(`overrides:${channelId}`, () =>
      Array.from(this.#channelOverrides.values()).filter((o) => o.channel === channelId),
    );
  }

  /** Topic `overrides:<categoryId>`. */
  categoryOverrides(categoryId: string): readonly CategoryOverride[] {
    return this.#memoized(`overrides:${categoryId}`, () =>
      Array.from(this.#categoryOverrides.values()).filter((o) => o.category === categoryId),
    );
  }

  /**
   * Topic `access:<communityId>`: what `userId` (the caller by default) may do across a
   * community, or `null` when they are not known to be a member.
   */
  access(communityId: string, userId?: string): CommunityPermissions | null {
    const compute = (user: string | null): CommunityPermissions | null => {
      const community = this.#communities.get(communityId);
      if (community === undefined || user === null) {
        return null;
      }
      const holds = this.#memberRoles.get(`${communityId}/${user}`);
      const owner = community.owner === user;
      if (holds === undefined && !owner) {
        return null;
      }
      return resolveCommunity(this.roles(communityId), holds ?? [], owner);
    };
    if (userId !== undefined && userId !== this.#myUserId) {
      return compute(userId);
    }
    return this.#memoized(`access:${communityId}`, () => compute(this.#myUserId));
  }

  /**
   * What `userId` may do in a channel, given what they may do across its community: its
   * category's overrides and then its own, or its parent's for a thread.
   */
  permissionsIn(channelId: string, access: CommunityPermissions): PermissionSet {
    const channel = this.#channels.get(channelId);
    const governing =
      channel?.parentChannel != null ? this.#channels.get(channel.parentChannel) : channel;
    if (governing === undefined) {
      return NO_PERMISSIONS;
    }
    const category = governing.parentCategory;
    return access.inChannel(
      category == null ? EMPTY_OVERRIDES : this.categoryOverrides(category),
      this.channelOverrides(governing.id),
    );
  }

  /**
   * Topic `channelAccess:<channelId>`: what the caller may do in a channel. In a DM (or a
   * thread in one) that is every channel permission; in a channel of a community they are not
   * known to be in, nothing.
   */
  channelAccess(channelId: string): PermissionSet {
    return this.#memoized(`channelAccess:${channelId}`, () => {
      const channel = this.#channels.get(channelId);
      if (channel === undefined) {
        return NO_PERMISSIONS;
      }
      if (channel.community == null) {
        return DM_PERMISSIONS;
      }
      const access = this.access(channel.community);
      return access === null ? NO_PERMISSIONS : this.permissionsIn(channelId, access);
    });
  }

  /** Topic `pins:<channelId>`: the channel's pins in their order, or `undefined` until loaded. */
  pins(channelId: string): readonly Pin[] | undefined {
    return this.#memoized(`pins:${channelId}`, () => {
      const pins = this.#pins.get(channelId);
      return pins === undefined
        ? undefined
        : Array.from(pins.values()).sort((a, b) => a.sortIndex - b.sortIndex);
    });
  }

  /** Installs a channel's pins as read, replacing what was held. */
  setPins(channelId: string, pins: readonly Pin[]): void {
    this.#batch(() => {
      this.#pins.set(channelId, new Map(pins.map((p) => [p.messageId, p])));
      this.#touch(`pins:${channelId}`);
    });
  }

  /** Topic `icon:<id>`. */
  icon(id: string): Icon | undefined {
    return this.#icons.get(id);
  }

  /** Caches an icon record read or uploaded by the caller. Icons never change once confirmed. */
  putIcon(icon: Icon): void {
    this.#batch(() => {
      this.#icons.set(icon.id, icon);
      this.#touch(`icon:${icon.id}`);
    });
  }

  /** Topic `poll:<id>`. */
  poll(id: string): Poll | undefined {
    return this.#polls.get(id);
  }

  /** Topic `poll:<id>`: the options the calling user has voted for on the poll. */
  myVotes(pollId: string): ReadonlySet<number> {
    return this.#myVotes.get(pollId) ?? EMPTY_VOTES;
  }

  /** Topic `poll:<id>`: the options the calling user wrote in on the poll and still stand. */
  myWriteIns(pollId: string): ReadonlySet<number> {
    return this.#myWriteIns.get(pollId) ?? EMPTY_VOTES;
  }

  /** Topic `read:<channelId>`: how far the caller has read the channel, if it is tracked. */
  readState(channelId: string): ReadState | undefined {
    return this.#readStates.get(channelId);
  }

  /** Topic `read:<channelId>`: whether the channel holds a message by someone else not yet read. */
  unread(channelId: string): boolean {
    const state = this.#readStates.get(channelId);
    return state?.lastMessage != null && state.lastMessage > state.lastRead;
  }

  /** Topic `mute:<channelId>`: the caller's mute of the channel while it lasts, if any. */
  mute(channelId: string): ChannelMute | undefined {
    return this.#mutes.get(channelId);
  }

  /** Topic `admin`: whether the caller may open the Administration Dashboard. */
  admin(): boolean {
    return this.#admin;
  }

  /** Records whether the caller may open the Administration Dashboard, as the server says. */
  setAdmin(admin: boolean): void {
    this.#batch(() => {
      if (this.#admin !== admin) {
        this.#admin = admin;
        this.#touch("admin");
      }
    });
  }

  /** Topic `collapse:<categoryId>`: whether the caller has the category collapsed. */
  collapsed(categoryId: string): boolean {
    return this.#collapsed.has(categoryId);
  }

  /**
   * Whether a channel stays in view under its collapsed category: while it is unread and not
   * muted, or its call has someone in it. Reads topics `read:<channelId>`, `mute:<channelId>`,
   * and `voice:<channelId>`; `unread` covers the first two.
   */
  shownWhenCollapsed(channelId: string): boolean {
    return (
      (this.unread(channelId) && !this.#mutes.has(channelId)) ||
      this.channelVoice(channelId).participants.length > 0
    );
  }

  /** When the first timed mute ends, as milliseconds since the epoch; `null` with none. */
  nextMuteEnd(): number | null {
    let next: number | null = null;
    for (const mute of this.#mutes.values()) {
      if (mute.until != null) {
        const end = Date.parse(mute.until);
        next = next === null ? end : Math.min(next, end);
      }
    }
    return next;
  }

  /**
   * Topic `unread`: the communities with an unread channel, and `UNREAD_DMS` when a DM is
   * unread. A muted channel counts for neither.
   */
  unreadPlaces(): ReadonlySet<string> {
    return this.#memoized("unread", () => {
      const places = new Set<string>();
      for (const state of this.#readStates.values()) {
        const channel = this.#channels.get(state.channel);
        if (
          channel !== undefined &&
          !this.#mutes.has(state.channel) &&
          this.unread(state.channel)
        ) {
          places.add(channel.community ?? UNREAD_DMS);
        }
      }
      return places;
    });
  }

  /** The tracked channels whose newest message by someone else is `messageId`. */
  channelsLastMessaged(messageId: string): string[] {
    const channels: string[] = [];
    for (const state of this.#readStates.values()) {
      if (state.lastMessage === messageId) {
        channels.push(state.channel);
      }
    }
    return channels;
  }

  /**
   * Topic `communities`: the calling user's communities in the order they arranged them, by
   * their membership's sort index, then by name.
   */
  communities(): readonly Community[] {
    return this.#memoized("communities", () => {
      const list: Community[] = [];
      for (const id of this.#myCommunities) {
        const community = this.#communities.get(id);
        if (community !== undefined) {
          list.push(community);
        }
      }
      return list.sort(
        (a, b) =>
          (this.#myOrder.get(a.id) ?? 0) - (this.#myOrder.get(b.id) ?? 0) ||
          a.name.localeCompare(b.name),
      );
    });
  }

  /** Where a community sits in the caller's list, as last told by the server or set locally. */
  communityOrder(communityId: string): number {
    return this.#myOrder.get(communityId) ?? 0;
  }

  /**
   * Records the caller's own arrangement of a community, as a drag and drop has just decided
   * it; the membership update event that follows is then a no-op.
   */
  setCommunityOrder(communityId: string, sortIndex: number): void {
    this.#batch(() => {
      this.#setMyOrder(communityId, sortIndex);
    });
  }

  /** Topic `channels:<communityId>`: every channel of the community, in sort order. */
  channels(communityId: string): readonly Channel[] {
    return this.#memoized(`channels:${communityId}`, () => {
      const list: Channel[] = [];
      for (const channel of this.#channels.values()) {
        // A thread records its community too, but belongs under its parent channel.
        if (channel.community === communityId && channel.ty !== "thread") {
          list.push(channel);
        }
      }
      return list.sort(compareBySortIndex);
    });
  }

  /**
   * Topic `dms`: the caller's DMs and group DMs, those with activity seen since they were
   * listed first (newest first), then the rest in the server's order.
   */
  dms(): readonly Channel[] {
    return this.#memoized("dms", () => {
      const list: Channel[] = [];
      for (const channel of this.#channels.values()) {
        if (isDm(channel)) {
          list.push(channel);
        }
      }
      const listed = new Map(this.#dmOrder.map((id, index) => [id, index]));
      return list.sort((a, b) => {
        const activeA = this.#dmActivity.get(a.id);
        const activeB = this.#dmActivity.get(b.id);
        if (activeA !== undefined || activeB !== undefined) {
          if (activeA === undefined) {
            return 1;
          }
          if (activeB === undefined) {
            return -1;
          }
          return activeB.localeCompare(activeA);
        }
        return (listed.get(a.id) ?? Infinity) - (listed.get(b.id) ?? Infinity);
      });
    });
  }

  /**
   * Topic `people`: the ids of everyone in the caller's communities besides the caller, from
   * the member lists read so far, sorted.
   */
  people(): readonly string[] {
    return this.#memoized("people", () => {
      const ids = new Set<string>();
      for (const communityId of this.#myCommunities) {
        for (const userId of this.#members.get(communityId) ?? []) {
          if (userId !== this.#myUserId) {
            ids.add(userId);
          }
        }
      }
      return Array.from(ids).sort();
    });
  }

  /** Topic `categories:<communityId>`, in sort order. */
  categories(communityId: string): readonly Category[] {
    return this.#memoized(`categories:${communityId}`, () => {
      const list: Category[] = [];
      for (const category of this.#categories.values()) {
        if (category.community === communityId) {
          list.push(category);
        }
      }
      return list.sort(compareBySortIndex);
    });
  }

  /**
   * Topic `members:<communityId>`: ids of the members the server has told us about, in the
   * order it listed them (most recently seen first).
   */
  memberIds(communityId: string): readonly string[] {
    return this.#memoized(
      `members:${communityId}`,
      () => Array.from(this.#members.get(communityId) ?? EMPTY_IDS) as readonly string[],
    );
  }

  /**
   * Topic `members:<communityId>`: the member records themselves, in the same order. The topic
   * fires when the membership changes and when any listed member's record changes, so a list
   * grouped by online status regroups on a status event.
   */
  members(communityId: string): readonly User[] {
    return this.#memoized(`members-records:${communityId}`, () => {
      const list: User[] = [];
      for (const id of this.#members.get(communityId) ?? EMPTY_IDS) {
        const user = this.#users.get(id);
        if (user !== undefined) {
          list.push(user);
        }
      }
      return list;
    });
  }

  /** Topic `messages:<channelId>`; `undefined` until a window has been loaded. */
  messages(channelId: string): MessageWindow | undefined {
    return this.#windows.get(channelId);
  }

  /** Topic `reactions:<messageId>`: the message's reactions, emoji in the order first used. */
  reactions(messageId: string): Reactions {
    return this.#reactions.get(messageId) ?? EMPTY_REACTIONS;
  }

  /** Topic `invites:<communityId>`: the community's invites, newest first. */
  invites(communityId: string): readonly Invite[] {
    return this.#memoized(`invites:${communityId}`, () => {
      const list: Invite[] = [];
      for (const invite of this.#invites.values()) {
        if (invite.community === communityId) {
          list.push(invite);
        }
      }
      return list.sort((a, b) => b.createdAt.localeCompare(a.createdAt));
    });
  }

  /** Topic `invite:<code>`. */
  invite(code: string): Invite | undefined {
    return this.#invites.get(code);
  }

  // ---------------------------------------------------------------------------------------
  // Writes from REST

  /**
   * Installs the result of the bootstrap read (`GET /users/@me` and the caller's community list
   * with channels, categories, and members included) and reconciles the cache against it.
   * Communities, channels, categories, and member samples that the bootstrap did not mention
   * are dropped, because the bootstrap is complete for those types. Message windows are
   * discarded, since a resync means history may have gaps; users and attachments are kept,
   * since a stale one is harmless and will be patched by later reads and events.
   */
  setBootstrap(me: User, communities: readonly Community[], included: Included): void {
    this.#batch(() => {
      this.#myUserId = me.id;
      this.#putUser(me);
      this.#touch("me");

      const listed = new Set(communities.map((c) => c.id));
      for (const id of this.#myCommunities) {
        if (!listed.has(id)) {
          this.#removeCommunity(id);
        }
      }
      this.#myCommunities.clear();
      for (const community of communities) {
        this.#myCommunities.add(community.id);
        this.#putCommunity(community);
      }
      this.#touch("communities");

      const keepChannels = new Set((included.channels ?? []).map((c) => c.id));
      for (const channel of Array.from(this.#channels.values())) {
        if (
          channel.community != null &&
          listed.has(channel.community) &&
          !keepChannels.has(channel.id)
        ) {
          this.#removeChannel(channel.id);
        }
      }
      const keepCategories = new Set((included.categories ?? []).map((c) => c.id));
      for (const category of Array.from(this.#categories.values())) {
        if (listed.has(category.community) && !keepCategories.has(category.id)) {
          this.#removeCategory(category.id);
        }
      }
      if (included.userCommunities !== undefined) {
        for (const id of listed) {
          this.#replaceMembers(id, []);
        }
      }
      this.ingest(included);

      for (const channelId of this.#windows.keys()) {
        this.#touch(`messages:${channelId}`);
      }
      this.#windows.clear();
      for (const messageId of this.#messages.keys()) {
        this.#touch(`message:${messageId}`);
      }
      this.#messages.clear();
      for (const messageId of this.#reactions.keys()) {
        this.#touch(`reactions:${messageId}`);
      }
      this.#reactions.clear();
      for (const pollId of this.#polls.keys()) {
        this.#touch(`poll:${pollId}`);
      }
      this.#polls.clear();
      this.#myVotes.clear();
      this.#myWriteIns.clear();
      // Calls are re-read with the communities; anything not in the bootstrap is over.
      for (const session of this.#voiceSessions.values()) {
        this.#touch(`voice:${session.channel}`);
      }
      this.#voiceSessions.clear();
      this.#voiceParticipants.clear();
      this.ingest({
        voiceSessions: included.voiceSessions ?? [],
        voiceParticipants: included.voiceParticipants ?? [],
      });
    });
  }

  /** Stores the records a sideloading read returned under `included`. */
  ingest(included: Included): void {
    this.#batch(() => {
      for (const community of included.communities ?? []) {
        this.#putCommunity(community);
      }
      for (const user of included.users ?? []) {
        this.#putUser(user);
      }
      for (const channel of included.channels ?? []) {
        this.#putChannel(channel);
      }
      // Messages outside any loaded window, such as the thread replies echoes show.
      for (const message of included.messages ?? []) {
        this.#putMessage(message);
      }
      for (const category of included.categories ?? []) {
        this.#putCategory(category);
      }
      for (const attachment of included.attachments ?? []) {
        this.#attachments.set(attachment.id, attachment);
        this.#touch(`attachment:${attachment.id}`);
      }
      for (const poll of included.polls ?? []) {
        this.#putPoll(poll);
      }
      for (const state of included.readStates ?? []) {
        this.#putReadState(state);
      }
      for (const mute of included.channelMutes ?? []) {
        this.#putMute(mute);
      }
      for (const { category } of included.categoryCollapses ?? []) {
        this.#setCollapsed(category, true);
      }
      if (included.roles !== undefined) {
        this.#replaceRoles(
          included.roles,
          included.channelOverrides ?? [],
          included.categoryOverrides ?? [],
        );
      }
      for (const session of included.voiceSessions ?? []) {
        this.#putVoiceSession(session);
      }
      // A read that brings participants brings all of them for the sessions it brings.
      if (included.voiceParticipants !== undefined) {
        for (const session of included.voiceSessions ?? []) {
          this.#voiceParticipants.set(session.id, new Map());
        }
        for (const participant of included.voiceParticipants) {
          this.#putVoiceParticipant(participant);
        }
        for (const session of included.voiceSessions ?? []) {
          this.#touch(`voice:${session.channel}`);
        }
      }
      // The caller's votes come with the polls they are on, and a poll with no vote listed
      // is one they have not voted on.
      if (included.pollVotes !== undefined) {
        const byPoll = new Map<string, Set<number>>();
        for (const poll of included.polls ?? []) {
          byPoll.set(poll.id, new Set());
        }
        for (const vote of included.pollVotes) {
          let options = byPoll.get(vote.poll);
          if (options === undefined) {
            options = new Set();
            byPoll.set(vote.poll, options);
          }
          options.add(vote.option);
        }
        for (const [pollId, options] of byPoll) {
          this.#setMyVotes(pollId, options);
        }
        // The caller's own write-ins come the same way, with the same polls.
        const written = new Map<string, Set<number>>();
        for (const poll of included.polls ?? []) {
          written.set(poll.id, new Set());
        }
        for (const own of included.ownWriteIns ?? []) {
          let options = written.get(own.poll);
          if (options === undefined) {
            options = new Set();
            written.set(own.poll, options);
          }
          options.add(own.option);
        }
        for (const [pollId, options] of written) {
          this.#myWriteIns.set(pollId, options);
          this.#touch(`poll:${pollId}`);
        }
      }
      const memberships = included.userCommunities;
      if (memberships !== undefined) {
        const byCommunity = new Map<string, string[]>();
        for (const membership of memberships) {
          let ids = byCommunity.get(membership.community);
          if (ids === undefined) {
            ids = [];
            byCommunity.set(membership.community, ids);
          }
          ids.push(membership.user);
          this.#setMemberRoles(membership.community, membership.user, membership.roles);
          if (membership.user === this.#myUserId) {
            this.#setMyOrder(membership.community, membership.sortIndex);
          }
        }
        for (const [communityId, userIds] of byCommunity) {
          this.#replaceMembers(communityId, userIds);
        }
      }
    });
  }

  /**
   * Installs a freshly read window of a channel's history, replacing whatever was loaded. Pass
   * the messages in any order.
   */
  replaceWindow(
    channelId: string,
    messages: readonly Message[],
    flags: { hasOlder: boolean; atLatest: boolean },
  ): void {
    this.#batch(() => {
      const ids = messages.map((m) => m.id).sort();
      for (const message of messages) {
        this.#putMessage(message);
      }
      this.#windows.set(channelId, { ids, ...flags });
      this.#touch(`messages:${channelId}`);
    });
  }

  /**
   * Adds messages older than the window's oldest, as returned by a `before` read. Over
   * `WINDOW_MAX_MESSAGES`, the newest messages are dropped and the window is no longer at the
   * latest.
   */
  prependWindow(channelId: string, messages: readonly Message[], hasOlder: boolean): void {
    this.#batch(() => {
      const window = this.#windows.get(channelId);
      if (window === undefined) {
        return;
      }
      const older = messages.map((m) => m.id).sort();
      for (const message of messages) {
        this.#putMessage(message);
      }
      let ids = older.concat(window.ids.filter((id) => !older.includes(id)));
      let atLatest = window.atLatest;
      if (ids.length > WINDOW_MAX_MESSAGES) {
        for (const id of ids.slice(WINDOW_MAX_MESSAGES)) {
          this.#evictMessage(id);
        }
        ids = ids.slice(0, WINDOW_MAX_MESSAGES);
        atLatest = false;
      }
      this.#windows.set(channelId, { ids, hasOlder, atLatest });
      this.#touch(`messages:${channelId}`);
    });
  }

  /**
   * Adds messages newer than the window's newest, as returned by an `after` read. Over
   * `WINDOW_MAX_MESSAGES`, the oldest messages are dropped and there are older ones to read.
   */
  appendWindow(channelId: string, messages: readonly Message[], atLatest: boolean): void {
    this.#batch(() => {
      const window = this.#windows.get(channelId);
      if (window === undefined) {
        return;
      }
      const newer = messages.map((m) => m.id).sort();
      for (const message of messages) {
        this.#putMessage(message);
      }
      let ids = window.ids.filter((id) => !newer.includes(id)).concat(newer);
      let hasOlder = window.hasOlder;
      if (ids.length > WINDOW_MAX_MESSAGES) {
        const excess = ids.length - WINDOW_MAX_MESSAGES;
        for (const id of ids.slice(0, excess)) {
          this.#evictMessage(id);
        }
        ids = ids.slice(excess);
        hasOlder = true;
      }
      this.#windows.set(channelId, { ids, hasOlder, atLatest });
      this.#touch(`messages:${channelId}`);
    });
  }

  /**
   * Caches a message the caller just created, appending it to the channel's window when that
   * window is at the latest. If the event stream delivered the message first, the stored copy
   * is kept: events published while the request was in flight (a link preview fetched before
   * the response returned, for instance) are newer than the response.
   */
  addMessage(message: Message): void {
    this.#batch(() => {
      if (!this.#messages.has(message.id)) {
        this.#putMessage(message);
      }
      this.#appendToWindow(message);
      this.#noteDmActivity(message.channelId, message.id);
    });
  }

  /**
   * Caches a poll the caller just opened, unless the stream delivered it first, in which case
   * the streamed copy may already carry votes and is kept.
   */
  addPoll(poll: Poll): void {
    this.#batch(() => {
      if (!this.#polls.has(poll.id)) {
        this.#putPoll(poll);
        this.#setMyVotes(poll.id, new Set());
      }
    });
  }

  /**
   * Records the caller's own vote as cast or withdrawn. The tally itself arrives by event; this
   * only tracks which options are theirs, which an anonymous poll's record never says.
   */
  setMyVote(pollId: string, option: number, voted: boolean): void {
    this.#batch(() => {
      const poll = this.#polls.get(pollId);
      const current = this.myVotes(pollId);
      if (current.has(option) === voted) {
        return;
      }
      const next = voted && poll?.multipleChoice === false ? new Set([option]) : new Set(current);
      if (voted) {
        next.add(option);
      } else {
        next.delete(option);
      }
      this.#setMyVotes(pollId, next);
    });
  }

  /** Records that the caller's write-in at `option` was added or removed. */
  /**
   * Records that the caller has read `channelId` up to `messageId`, ahead of the server's
   * `channelRead`. A position only moves forward.
   */
  setLastRead(channelId: string, messageId: string): void {
    this.#batch(() => {
      const state = this.#readStates.get(channelId);
      if (state !== undefined && messageId > state.lastRead) {
        this.#putReadState({ ...state, lastRead: messageId });
      }
    });
  }

  /**
   * Replaces every mute held with `mutes`, the complete list a bootstrap read, so a mute lifted
   * while the stream was away does not linger.
   */
  replaceMutes(mutes: readonly ChannelMute[]): void {
    this.#batch(() => {
      for (const channelId of Array.from(this.#mutes.keys())) {
        this.#removeMute(channelId);
      }
      for (const mute of mutes) {
        this.#putMute(mute);
      }
    });
  }

  /**
   * Replaces every collapsed category held with `categories`, the complete list a bootstrap
   * read, so one expanded while the stream was away does not stay folded.
   */
  replaceCollapsed(categories: readonly string[]): void {
    this.#batch(() => {
      for (const category of Array.from(this.#collapsed)) {
        this.#setCollapsed(category, false);
      }
      for (const category of categories) {
        this.#setCollapsed(category, true);
      }
    });
  }

  /** Ends the mutes whose time is up at `now` (milliseconds since the epoch). */
  expireMutes(now: number): void {
    this.#batch(() => {
      for (const mute of Array.from(this.#mutes.values())) {
        if (mute.until != null && Date.parse(mute.until) <= now) {
          this.#removeMute(mute.channel);
        }
      }
    });
  }

  /** Stores a read state the server sent for one channel, replacing what was held. */
  putReadState(state: ReadState): void {
    this.#batch(() => {
      this.#putReadState(state);
    });
  }

  setMyWriteIn(pollId: string, option: number, mine: boolean): void {
    this.#batch(() => {
      const next = new Set(this.myWriteIns(pollId));
      if (mine) {
        next.add(option);
      } else {
        next.delete(option);
      }
      this.#myWriteIns.set(pollId, next);
      this.#touch(`poll:${pollId}`);
    });
  }

  /**
   * Drops the caller's votes and write-ins on answers the poll no longer offers, which anyone
   * may have removed: a removed write-in keeps its index but loses its votes.
   */
  #forgetRemovedWriteIns(poll: Poll): void {
    const removed = (option: number) =>
      option >= poll.options.length && poll.writeIns[option - poll.options.length] == null;
    const votes = this.myVotes(poll.id);
    if (Array.from(votes).some(removed)) {
      this.#setMyVotes(poll.id, new Set(Array.from(votes).filter((o) => !removed(o))));
    }
    const writeIns = this.myWriteIns(poll.id);
    if (Array.from(writeIns).some(removed)) {
      this.#myWriteIns.set(poll.id, new Set(Array.from(writeIns).filter((o) => !removed(o))));
      this.#touch(`poll:${poll.id}`);
    }
  }

  /** Installs a community's invite list as read from the server, replacing what was held. */
  replaceInvites(communityId: string, invites: readonly Invite[]): void {
    this.#batch(() => {
      for (const invite of Array.from(this.#invites.values())) {
        if (invite.community === communityId) {
          this.#removeInvite(invite.code);
        }
      }
      for (const invite of invites) {
        this.#putInvite(invite);
      }
    });
  }

  /** Stores an invite the caller just created; the matching event is then a no-op. */
  upsertInvite(invite: Invite): void {
    this.#batch(() => {
      this.#putInvite(invite);
    });
  }

  /** Drops an invite the caller just revoked; the matching event is then a no-op. */
  removeInvite(code: string): void {
    this.#batch(() => {
      this.#removeInvite(code);
    });
  }

  /** Forgets everything, for sign-out. */
  /**
   * Installs the caller's DMs as the server listed them, most recently active first, dropping
   * any the cache held that the list no longer has.
   */
  setDms(dms: readonly Channel[]): void {
    this.#batch(() => {
      const listed = new Set(dms.map((dm) => dm.id));
      for (const channel of Array.from(this.#channels.values())) {
        if (isDm(channel) && !listed.has(channel.id)) {
          this.#removeChannel(channel.id);
        }
      }
      this.#dmOrder = dms.map((dm) => dm.id);
      this.#dmActivity.clear();
      for (const dm of dms) {
        this.#putChannel(dm);
      }
      this.#touch("dms");
    });
  }

  clear(): void {
    this.#batch(() => {
      for (const topic of this.#listeners.keys()) {
        this.#touch(topic);
      }
      this.#users.clear();
      this.#communities.clear();
      this.#channels.clear();
      this.#categories.clear();
      this.#messages.clear();
      this.#attachments.clear();
      this.#icons.clear();
      this.#voiceSessions.clear();
      this.#voiceParticipants.clear();
      this.#members.clear();
      this.#memberOf.clear();
      this.#reactions.clear();
      this.#polls.clear();
      this.#myVotes.clear();
      this.#myWriteIns.clear();
      this.#readStates.clear();
      this.#mutes.clear();
      this.#collapsed.clear();
      this.#roles.clear();
      this.#pins.clear();
      this.#channelOverrides.clear();
      this.#categoryOverrides.clear();
      this.#memberRoles.clear();
      this.#admin = false;
      this.#windows.clear();
      this.#invites.clear();
      this.#myCommunities.clear();
      this.#myOrder.clear();
      this.#dmOrder = [];
      this.#dmActivity.clear();
      this.#removedChannels.clear();
      this.#myUserId = null;
      this.#memo.clear();
    });
  }

  // ---------------------------------------------------------------------------------------
  // Writes from the event stream

  /**
   * Applies one server event. Updates for records the cache does not hold are ignored: there is
   * nothing to patch, and the record will arrive whole from the next read that needs it.
   * Deletes cascade locally, so a deleted community takes its channels, categories, and
   * messages with it without waiting for their own events.
   */
  applyEvent(event: ServerEvent): void {
    this.#batch(() => {
      switch (event.serverEvent) {
        case "user":
          if (event.type === "create") {
            this.#putUser(created(event));
          } else if (event.type === "update") {
            const user = this.#users.get(event.id);
            if (user !== undefined) {
              this.#putUser(mergePatch(user, event));
            }
          } else {
            this.#removeUser(event.id);
          }
          break;
        case "community":
          if (event.type === "create") {
            this.#putCommunity(created(event));
          } else if (event.type === "update") {
            const community = this.#communities.get(event.id);
            if (community !== undefined) {
              this.#putCommunity(mergePatch(community, event));
            }
          } else {
            this.#removeCommunity(event.id);
          }
          break;
        case "role":
          if (event.type === "create") {
            this.#putRole(created(event));
          } else if (event.type === "update") {
            const role = this.#roles.get(event.id);
            if (role !== undefined) {
              this.#putRole(mergePatch(role, event));
            }
          } else {
            this.#removeRole(event.id);
          }
          break;
        case "channelOverride":
          if (event.type === "delete") {
            this.#removeChannelOverride(event.channel, event.role);
          } else {
            const key = `${event.channel}/${event.role}`;
            const current = this.#channelOverrides.get(key) ?? {
              channel: event.channel,
              role: event.role,
              allow: [],
              deny: [],
            };
            this.#putChannelOverride(mergePatch(current, event));
          }
          break;
        case "categoryOverride":
          if (event.type === "delete") {
            this.#removeCategoryOverride(event.category, event.role);
          } else {
            const key = `${event.category}/${event.role}`;
            const current = this.#categoryOverrides.get(key) ?? {
              category: event.category,
              role: event.role,
              allow: [],
              deny: [],
            };
            this.#putCategoryOverride(mergePatch(current, event));
          }
          break;
        case "userCommunity":
          if (event.type === "create") {
            this.#addMember(event.community, event.user);
            this.#setMemberRoles(event.community, event.user, event.roles);
            if (event.user === this.#myUserId) {
              this.#myCommunities.add(event.community);
              this.#setMyOrder(event.community, event.sortIndex);
              this.#touch("communities");
            }
          } else if (event.type === "update") {
            if (event.user === this.#myUserId && event.sortIndex != null) {
              this.#setMyOrder(event.community, event.sortIndex);
            }
            if (event.roles != null) {
              this.#setMemberRoles(event.community, event.user, event.roles);
            }
          } else {
            this.#removeMember(event.community, event.user);
            this.#setMemberRoles(event.community, event.user, undefined);
            if (event.user === this.#myUserId) {
              this.#myCommunities.delete(event.community);
              this.#touch("communities");
            }
          }
          break;
        case "channel":
          if (event.type === "create") {
            this.#putChannel(created(event));
          } else if (event.type === "update") {
            const channel = this.#channels.get(event.id);
            if (channel !== undefined) {
              const updated = mergePatch(channel, event);
              // A DM whose recipients no longer include the caller is one they left.
              if (
                isDm(updated) &&
                this.#myUserId !== null &&
                !updated.recipients.includes(this.#myUserId)
              ) {
                this.#removeChannel(updated.id);
              } else {
                this.#putChannel(updated);
              }
            }
          } else {
            this.#removeChannel(event.id);
          }
          break;
        case "category":
          if (event.type === "create") {
            this.#putCategory(created(event));
          } else if (event.type === "update") {
            const category = this.#categories.get(event.id);
            if (category !== undefined) {
              this.#putCategory(mergePatch(category, event));
            }
          } else {
            this.#removeCategory(event.id);
          }
          break;
        case "message":
          if (event.type === "create") {
            const message = created(event);
            this.#putMessage(message);
            this.#appendToWindow(message);
            this.#noteDmActivity(message.channelId, message.id);
            this.#noteNewMessage(message);
          } else if (event.type === "update") {
            const message = this.#messages.get(event.id);
            if (message !== undefined) {
              this.#putMessage(mergePatch(message, event));
            }
          } else {
            this.#removeMessage(event.id);
          }
          break;
        case "react":
          this.#applyReaction(event.messageId, event.emoji, event.userId, event.type === "create");
          break;
        case "invite":
          if (event.type === "create") {
            this.#putInvite(created(event));
          } else if (event.type === "update") {
            const invite = this.#invites.get(event.code);
            if (invite !== undefined) {
              this.#putInvite(mergePatch(invite, event));
            }
          } else {
            this.#removeInvite(event.code);
          }
          break;
        case "poll":
          if (event.type === "create") {
            this.#putPoll(created(event));
          } else if (event.type === "update") {
            const poll = this.#polls.get(event.id);
            if (poll !== undefined) {
              const next = mergePatch(poll, event);
              this.#putPoll(next);
              if (event.writeIns != null) {
                this.#forgetRemovedWriteIns(next);
              }
            }
          } else {
            this.#removePoll(event.id);
          }
          break;
        case "voiceSession":
          if (event.type === "create") {
            this.#putVoiceSession(created(event));
          } else {
            this.#removeVoiceSession(event.id);
          }
          break;
        case "voiceParticipant":
          if (event.type === "create") {
            this.#putVoiceParticipant(created(event));
          } else if (event.type === "update") {
            const current = this.#voiceParticipants.get(event.session)?.get(event.user);
            if (current !== undefined) {
              this.#setVoiceParticipant({
                ...current,
                muted: event.muted ?? current.muted,
                sharingScreen: event.sharingScreen ?? current.sharingScreen,
                deafened: event.deafened ?? current.deafened,
              });
            }
          } else {
            this.#removeVoiceParticipant(event.session, event.user);
          }
          break;
        case "categoryCollapseChanged":
          this.#setCollapsed(event.category, event.collapsed);
          break;
        case "channelMuteChanged":
          if (event.muted) {
            this.#putMute({ channel: event.channel, until: event.until ?? null });
          } else {
            this.#removeMute(event.channel);
          }
          break;
        case "channelRead": {
          const state = this.#readStates.get(event.channel);
          if (state !== undefined && event.lastRead > state.lastRead) {
            this.#putReadState({ ...state, lastRead: event.lastRead });
          }
          break;
        }
        case "voiceSpeaking": {
          const session = Array.from(this.#voiceSessions.values()).find(
            (s) => s.channel === event.channel,
          );
          const current = session && this.#voiceParticipants.get(session.id)?.get(event.user);
          if (current !== undefined && current.speaking !== event.speaking) {
            this.#setVoiceParticipant({
              ...current,
              speaking: event.speaking,
              lastSpokeAt: event.speaking ? this.#now() : current.lastSpokeAt,
            });
          }
          break;
        }
        case "voiceSessionEnded":
          // The reason is for whoever was in the call; the session's own delete follows.
          break;
        case "pin":
          if (event.type === "create") {
            // A pin names only its message; it is placed in that message's channel, when the
            // message and the channel's pins are both held.
            const channelId = this.#messages.get(event.messageId)?.channelId;
            const pins = channelId === undefined ? undefined : this.#pins.get(channelId);
            if (channelId !== undefined && pins !== undefined) {
              pins.set(event.messageId, created(event));
              this.#touch(`pins:${channelId}`);
            }
          } else if (event.type === "delete") {
            for (const [channelId, pins] of this.#pins) {
              if (pins.delete(event.messageId)) {
                this.#touch(`pins:${channelId}`);
              }
            }
          }
          break;
      }
    });
  }

  // ---------------------------------------------------------------------------------------
  // Internals

  #memoized<T>(topic: Topic, compute: () => T): T {
    if (this.#memo.has(topic)) {
      return this.#memo.get(topic) as T;
    }
    const value = compute();
    this.#memo.set(topic, value);
    return value;
  }

  #touch(topic: Topic): void {
    this.#memo.delete(topic);
    this.#dirty.add(topic);
  }

  #batch(write: () => void): void {
    this.#batchDepth += 1;
    try {
      write();
    } finally {
      this.#batchDepth -= 1;
      if (this.#batchDepth === 0) {
        this.#flush();
      }
    }
  }

  #flush(): void {
    const topics = Array.from(this.#dirty);
    this.#dirty.clear();
    for (const topic of topics) {
      const listeners = this.#listeners.get(topic);
      if (listeners !== undefined) {
        for (const listener of Array.from(listeners)) {
          listener();
        }
      }
    }
  }

  /**
   * The presence of users, as `GET /users/statuses` answered. Presence is pulled for the users
   * on screen rather than pushed, so this is the only way it changes after a bootstrap.
   */
  applyStatuses(statuses: readonly { id: string; onlineStatus: UserOnlineStatus }[]): void {
    this.#batch(() => {
      for (const { id, onlineStatus } of statuses) {
        const user = this.#users.get(id);
        if (user !== undefined && user.onlineStatus !== onlineStatus) {
          this.#putUser({ ...user, onlineStatus });
        }
      }
    });
  }

  /**
   * The users whose presence is worth asking for: the members shown for every community the
   * user is in, and everyone in a call.
   */
  presenceCandidates(): string[] {
    const ids = new Set<string>();
    for (const community of this.communities()) {
      for (const id of this.memberIds(community.id)) {
        ids.add(id);
      }
      for (const channel of this.channels(community.id)) {
        for (const participant of this.channelVoice(channel.id).participants) {
          ids.add(participant.user);
        }
      }
    }
    return Array.from(ids);
  }

  #putUser(user: User): void {
    this.#users.set(user.id, user);
    this.#touch(`user:${user.id}`);
    if (user.id === this.#myUserId) {
      this.#touch("me");
    }
    for (const communityId of this.#memberOf.get(user.id) ?? []) {
      this.#touchMembers(communityId);
    }
  }

  /** Both member views share one topic; the record view has its own memo entry. */
  #touchMembers(communityId: string): void {
    this.#memo.delete(`members-records:${communityId}`);
    this.#touch(`members:${communityId}`);
    this.#touch("people");
  }

  #removeUser(id: string): void {
    if (!this.#users.delete(id)) {
      return;
    }
    this.#touch(`user:${id}`);
    for (const communityId of Array.from(this.#memberOf.get(id) ?? [])) {
      this.#removeMember(communityId, id);
    }
  }

  #putCommunity(community: Community): void {
    const previous = this.#communities.get(community.id);
    this.#communities.set(community.id, community);
    this.#touch(`community:${community.id}`);
    if (this.#myCommunities.has(community.id)) {
      this.#touch("communities");
    }
    if ((previous?.owner ?? null) !== (community.owner ?? null)) {
      this.#accessChanged(community.id);
    }
  }

  #removeCommunity(id: string): void {
    for (const channel of Array.from(this.#channels.values())) {
      if (channel.community === id) {
        this.#removeChannel(channel.id);
      }
    }
    for (const category of Array.from(this.#categories.values())) {
      if (category.community === id) {
        this.#removeCategory(category.id);
      }
    }
    this.#replaceMembers(id, []);
    this.#members.delete(id);
    for (const role of Array.from(this.#roles.values())) {
      if (role.community === id) {
        this.#roles.delete(role.id);
      }
    }
    for (const key of Array.from(this.#memberRoles.keys())) {
      if (key.startsWith(`${id}/`)) {
        this.#memberRoles.delete(key);
      }
    }
    this.#touch(`roles:${id}`);
    this.#touch(`access:${id}`);
    for (const invite of Array.from(this.#invites.values())) {
      if (invite.community === id) {
        this.#removeInvite(invite.code);
      }
    }
    if (this.#myCommunities.delete(id)) {
      this.#touch("communities");
    }
    if (this.#communities.delete(id)) {
      this.#touch(`community:${id}`);
    }
  }

  #putRole(role: Role): void {
    this.#roles.set(role.id, role);
    this.#accessChanged(role.community);
  }

  #removeRole(id: string): void {
    const role = this.#roles.get(id);
    if (role === undefined) {
      return;
    }
    this.#roles.delete(id);
    for (const [key, o] of Array.from(this.#channelOverrides)) {
      if (o.role === id) {
        this.#channelOverrides.delete(key);
        this.#touch(`overrides:${o.channel}`);
      }
    }
    for (const [key, o] of Array.from(this.#categoryOverrides)) {
      if (o.role === id) {
        this.#categoryOverrides.delete(key);
        this.#touch(`overrides:${o.category}`);
      }
    }
    for (const [key, held] of Array.from(this.#memberRoles)) {
      if (held.includes(id)) {
        this.#memberRoles.set(
          key,
          held.filter((r) => r !== id),
        );
      }
    }
    this.#accessChanged(role.community);
  }

  #putChannelOverride(o: ChannelOverride): void {
    this.#channelOverrides.set(`${o.channel}/${o.role}`, o);
    this.#overrideChanged(o.channel, this.#channels.get(o.channel)?.community);
  }

  #removeChannelOverride(channel: string, role: string): void {
    if (this.#channelOverrides.delete(`${channel}/${role}`)) {
      this.#overrideChanged(channel, this.#channels.get(channel)?.community);
    }
  }

  #putCategoryOverride(o: CategoryOverride): void {
    this.#categoryOverrides.set(`${o.category}/${o.role}`, o);
    this.#overrideChanged(o.category, this.#categories.get(o.category)?.community);
  }

  #removeCategoryOverride(category: string, role: string): void {
    if (this.#categoryOverrides.delete(`${category}/${role}`)) {
      this.#overrideChanged(category, this.#categories.get(category)?.community);
    }
  }

  #overrideChanged(target: string, community: string | null | undefined): void {
    this.#touch(`overrides:${target}`);
    if (community != null) {
      this.#accessChanged(community);
    }
  }

  /**
   * Installs a complete listing of some communities' roles and overrides, dropping what those
   * communities held that the listing lacks.
   */
  #replaceRoles(
    roles: readonly Role[],
    channelOverrides: readonly ChannelOverride[],
    categoryOverrides: readonly CategoryOverride[],
  ): void {
    const communities = new Set(roles.map((r) => r.community));
    for (const role of Array.from(this.#roles.values())) {
      if (communities.has(role.community)) {
        this.#roles.delete(role.id);
      }
    }
    for (const [key, o] of Array.from(this.#channelOverrides)) {
      const community = this.#channels.get(o.channel)?.community;
      if (community != null && communities.has(community)) {
        this.#channelOverrides.delete(key);
        this.#touch(`overrides:${o.channel}`);
      }
    }
    for (const [key, o] of Array.from(this.#categoryOverrides)) {
      const community = this.#categories.get(o.category)?.community;
      if (community !== undefined && communities.has(community)) {
        this.#categoryOverrides.delete(key);
        this.#touch(`overrides:${o.category}`);
      }
    }
    for (const role of roles) {
      this.#roles.set(role.id, role);
    }
    for (const o of channelOverrides) {
      this.#channelOverrides.set(`${o.channel}/${o.role}`, o);
      this.#touch(`overrides:${o.channel}`);
    }
    for (const o of categoryOverrides) {
      this.#categoryOverrides.set(`${o.category}/${o.role}`, o);
      this.#touch(`overrides:${o.category}`);
    }
    for (const community of communities) {
      this.#accessChanged(community);
    }
  }

  #setMemberRoles(communityId: string, userId: string, roles: readonly string[] | undefined): void {
    const key = `${communityId}/${userId}`;
    if (roles === undefined) {
      this.#memberRoles.delete(key);
    } else {
      this.#memberRoles.set(key, roles);
    }
    this.#touch(`roles:${communityId}`);
    if (userId === this.#myUserId) {
      this.#accessChanged(communityId);
    }
  }

  /**
   * Something that decides what the caller may do in a community changed: every answer that
   * depends on it is recomputed, and the channels they may no longer view are let go of, as
   * the server stops sending anything about them.
   */
  #accessChanged(communityId: string): void {
    this.#touch(`roles:${communityId}`);
    this.#touch(`access:${communityId}`);
    const channels = Array.from(this.#channels.values()).filter((c) => c.community === communityId);
    for (const channel of channels) {
      this.#touch(`channelAccess:${channel.id}`);
    }
    // Until both the community's roles and the caller's own are known, nothing is decided.
    const known =
      this.#memberRoles.has(`${communityId}/${this.#myUserId ?? ""}`) &&
      this.roles(communityId).some((r) => r.everyone);
    if (known) {
      for (const channel of channels) {
        if (!this.channelAccess(channel.id).has("viewChannel")) {
          this.#removeChannel(channel.id);
        }
      }
    }
  }

  #putChannel(channel: Channel): void {
    const previous = this.#channels.get(channel.id);
    this.#channels.set(channel.id, channel);
    this.#removedChannels.delete(channel.id);
    this.#touch(`channel:${channel.id}`);
    this.#touch(`channelAccess:${channel.id}`);
    // Where an unread channel counts depends on the channel, which may arrive after its state.
    if (this.#readStates.has(channel.id)) {
      this.#touch("unread");
    }
    if (isDm(channel)) {
      // A DM the listing did not have is new, so newer than everything listed.
      if (!this.#dmOrder.includes(channel.id) && !this.#dmActivity.has(channel.id)) {
        this.#dmActivity.set(channel.id, channel.id);
      }
      this.#touch("dms");
    }
    if (previous?.community != null) {
      this.#touch(`channels:${previous.community}`);
    }
    if (channel.community != null) {
      this.#touch(`channels:${channel.community}`);
    }
  }

  #setCollapsed(categoryId: string, collapsed: boolean): void {
    if (this.#collapsed.has(categoryId) === collapsed) {
      return;
    }
    if (collapsed) {
      this.#collapsed.add(categoryId);
    } else {
      this.#collapsed.delete(categoryId);
    }
    this.#touch(`collapse:${categoryId}`);
  }

  #putMute(mute: ChannelMute): void {
    this.#mutes.set(mute.channel, mute);
    this.#touch(`mute:${mute.channel}`);
    this.#touch("unread");
  }

  #removeMute(channelId: string): void {
    if (this.#mutes.delete(channelId)) {
      this.#touch(`mute:${channelId}`);
      this.#touch("unread");
    }
  }

  #putReadState(state: ReadState): void {
    this.#readStates.set(state.channel, state);
    this.#touch(`read:${state.channel}`);
    this.#touch("unread");
  }

  /**
   * Keeps read states current as messages arrive: someone else's message is the channel's
   * newest, and the caller's own is read, as the server records it. A channel with no read
   * state yet, one made since the caller's channels were last read, is unread from its start.
   * Threads keep no read state.
   */
  #noteNewMessage(message: Message): void {
    const channel = this.#channels.get(message.channelId);
    if (channel === undefined || channel.ty === "thread") {
      return;
    }
    const state = this.#readStates.get(message.channelId) ?? {
      channel: message.channelId,
      lastRead: "",
      lastMessage: null,
    };
    if (message.author === this.#myUserId) {
      if (message.id > state.lastRead) {
        this.#putReadState({ ...state, lastRead: message.id });
      }
    } else if (state.lastMessage == null || message.id > state.lastMessage) {
      this.#putReadState({ ...state, lastMessage: message.id });
    }
  }

  #removeChannel(id: string): void {
    const channel = this.#channels.get(id);
    if (channel === undefined) {
      return;
    }
    if (this.#readStates.delete(id)) {
      this.#touch(`read:${id}`);
      this.#touch("unread");
    }
    this.#removeMute(id);
    this.#channels.delete(id);
    this.#removedChannels.add(id);
    this.#touch(`channel:${id}`);
    this.#touch(`channelAccess:${id}`);
    if (isDm(channel)) {
      this.#dmOrder = this.#dmOrder.filter((other) => other !== id);
      this.#dmActivity.delete(id);
      this.#touch("dms");
    }
    for (const session of Array.from(this.#voiceSessions.values())) {
      if (session.channel === id) {
        this.#removeVoiceSession(session.id);
      }
    }
    if (channel.community != null) {
      this.#touch(`channels:${channel.community}`);
    }
    const window = this.#windows.get(id);
    if (window !== undefined) {
      for (const messageId of window.ids) {
        this.#removeMessage(messageId);
      }
      this.#windows.delete(id);
      this.#touch(`messages:${id}`);
    }
  }

  #putCategory(category: Category): void {
    const previous = this.#categories.get(category.id);
    this.#categories.set(category.id, category);
    this.#touch(`category:${category.id}`);
    if (previous !== undefined && previous.community !== category.community) {
      this.#touch(`categories:${previous.community}`);
    }
    this.#touch(`categories:${category.community}`);
  }

  #removeCategory(id: string): void {
    const category = this.#categories.get(id);
    if (category === undefined) {
      return;
    }
    this.#categories.delete(id);
    this.#touch(`category:${id}`);
    this.#touch(`categories:${category.community}`);
    // Channels filed under the category become top-level rather than disappearing.
    for (const channel of this.#channels.values()) {
      if (channel.parentCategory === id) {
        this.#putChannel({ ...channel, parentCategory: null });
      }
    }
  }

  #setMyOrder(communityId: string, sortIndex: number): void {
    if (this.#myOrder.get(communityId) !== sortIndex) {
      this.#myOrder.set(communityId, sortIndex);
      this.#touch("communities");
    }
  }

  /** Moves a DM up the list when a message arrives in it. */
  #noteDmActivity(channelId: string, messageId: string): void {
    const channel = this.#channels.get(channelId);
    if (channel === undefined || !isDm(channel)) {
      return;
    }
    const seen = this.#dmActivity.get(channelId);
    if (seen === undefined || messageId > seen) {
      this.#dmActivity.set(channelId, messageId);
      this.#touch("dms");
    }
  }

  #putMessage(message: Message): void {
    this.#messages.set(message.id, message);
    this.#touch(`message:${message.id}`);
  }

  #putVoiceSession(session: VoiceSession): void {
    this.#voiceSessions.set(session.id, session);
    if (!this.#voiceParticipants.has(session.id)) {
      this.#voiceParticipants.set(session.id, new Map());
    }
    this.#touch(`voice:${session.channel}`);
  }

  #removeVoiceSession(id: string): void {
    const session = this.#voiceSessions.get(id);
    if (session === undefined) {
      return;
    }
    this.#voiceSessions.delete(id);
    this.#voiceParticipants.delete(id);
    this.#touch(`voice:${session.channel}`);
  }

  #putVoiceParticipant(participant: VoiceParticipant): void {
    const current = this.#voiceParticipants.get(participant.session)?.get(participant.user);
    this.#setVoiceParticipant({
      user: participant.user,
      session: participant.session,
      channel: participant.channel,
      joinedAt: participant.joinedAt,
      muted: participant.muted,
      sharingScreen: participant.sharingScreen,
      deafened: participant.deafened,
      speaking: current?.speaking ?? false,
      lastSpokeAt: current?.lastSpokeAt ?? null,
    });
  }

  #setVoiceParticipant(participant: VoiceParticipantState): void {
    let bySession = this.#voiceParticipants.get(participant.session);
    if (bySession === undefined) {
      bySession = new Map();
      this.#voiceParticipants.set(participant.session, bySession);
    }
    bySession.set(participant.user, participant);
    this.#touch(`voice:${participant.channel}`);
  }

  #removeVoiceParticipant(session: string, user: string): void {
    const bySession = this.#voiceParticipants.get(session);
    const participant = bySession?.get(user);
    if (bySession === undefined || participant === undefined) {
      return;
    }
    bySession.delete(user);
    this.#touch(`voice:${participant.channel}`);
  }

  #putPoll(poll: Poll): void {
    this.#polls.set(poll.id, poll);
    this.#touch(`poll:${poll.id}`);
  }

  #setMyVotes(pollId: string, options: ReadonlySet<number>): void {
    this.#myVotes.set(pollId, options);
    this.#touch(`poll:${pollId}`);
  }

  #removePoll(id: string): void {
    const writeIns = this.#myWriteIns.delete(id);
    if (this.#polls.delete(id) || this.#myVotes.delete(id) || writeIns) {
      this.#touch(`poll:${id}`);
    }
  }

  /**
   * Installs the reactions a message read brought for `messageIds`, replacing what was held for
   * each; a message the read brought no summary for has none.
   */
  setReactions(messageIds: readonly string[], summaries: readonly ReactionSummary[]): void {
    this.#batch(() => {
      const byMessage = new Map<string, Map<string, EmojiReactions>>();
      for (const summary of summaries) {
        let byEmoji = byMessage.get(summary.messageId);
        if (byEmoji === undefined) {
          byEmoji = new Map();
          byMessage.set(summary.messageId, byEmoji);
        }
        byEmoji.set(summary.emoji, {
          count: summary.count,
          me: summary.me,
          users: summary.users,
        });
      }
      for (const id of messageIds) {
        const byEmoji = byMessage.get(id);
        if (byEmoji === undefined) {
          if (!this.#reactions.delete(id)) {
            continue;
          }
        } else {
          this.#reactions.set(id, byEmoji);
        }
        this.#touch(`reactions:${id}`);
      }
    });
  }

  /**
   * Counts one person's reaction in or out. The caller's own reaction is applied from the
   * request's answer and again from its event, so a second telling of it changes nothing;
   * anyone else's arrives once, as an event.
   */
  #applyReaction(messageId: string, emoji: string, userId: string, added: boolean): void {
    const current = this.#reactions.get(messageId) ?? EMPTY_REACTIONS;
    const summary = current.get(emoji);
    const mine = userId === this.#myUserId;
    const counted = summary !== undefined && (mine ? summary.me : summary.users.includes(userId));
    // Told twice. Whether someone else beyond the named few already reacted cannot be told,
    // but their events arrive once.
    if (added && counted) {
      return;
    }
    // The caller's own reaction, already gone.
    if (!added && mine && !counted) {
      return;
    }
    // A fresh map, so subscribers see a new reference, in the same emoji order.
    const next = new Map(current);
    if (added) {
      const base = summary ?? { count: 0, me: false, users: [] };
      next.set(emoji, {
        count: base.count + 1,
        me: base.me || mine,
        users: base.users.length < REACTION_SUMMARY_USERS ? [...base.users, userId] : base.users,
      });
    } else if (summary !== undefined) {
      if (summary.count <= 1) {
        next.delete(emoji);
      } else {
        next.set(emoji, {
          count: summary.count - 1,
          me: summary.me && !mine,
          users: summary.users.filter((id) => id !== userId),
        });
      }
    } else {
      return;
    }
    if (next.size === 0) {
      this.#reactions.delete(messageId);
    } else {
      this.#reactions.set(messageId, next);
    }
    this.#touch(`reactions:${messageId}`);
  }

  #removeMessage(id: string): void {
    const message = this.#messages.get(id);
    if (message === undefined) {
      return;
    }
    this.#messages.delete(id);
    this.#touch(`message:${id}`);
    if (this.#reactions.delete(id)) {
      this.#touch(`reactions:${id}`);
    }
    // Deleting the message a poll is shown in ends the poll on the server too.
    if (message.kind === "poll" && message.poll != null) {
      this.#removePoll(message.poll);
    }
    const window = this.#windows.get(message.channelId);
    if (window?.ids.includes(id)) {
      this.#windows.set(message.channelId, {
        ...window,
        ids: window.ids.filter((other) => other !== id),
      });
      this.#touch(`messages:${message.channelId}`);
    }
  }

  #appendToWindow(message: Message): void {
    const window = this.#windows.get(message.channelId);
    if (!window?.atLatest) {
      return;
    }
    const at = lowerBound(window.ids, message.id);
    if (window.ids[at] === message.id) {
      return;
    }
    let ids = window.ids.slice();
    ids.splice(at, 0, message.id);
    let hasOlder = window.hasOlder;
    // A window followed live for long enough would otherwise grow without bound.
    if (ids.length > WINDOW_MAX_MESSAGES) {
      for (const id of ids.slice(0, ids.length - WINDOW_MAX_MESSAGES)) {
        this.#evictMessage(id);
      }
      ids = ids.slice(ids.length - WINDOW_MAX_MESSAGES);
      hasOlder = true;
    }
    this.#windows.set(message.channelId, { ...window, ids, hasOlder });
    this.#touch(`messages:${message.channelId}`);
  }

  /**
   * Forgets a message that fell out of its window, with its reactions. Unlike a delete this
   * says nothing about the server; the message is read again when the window returns to it.
   */
  #evictMessage(id: string): void {
    if (this.#messages.delete(id)) {
      this.#touch(`message:${id}`);
    }
    if (this.#reactions.delete(id)) {
      this.#touch(`reactions:${id}`);
    }
  }

  #putInvite(invite: Invite): void {
    this.#invites.set(invite.code, invite);
    this.#touch(`invite:${invite.code}`);
    this.#touch(`invites:${invite.community}`);
  }

  #removeInvite(code: string): void {
    const invite = this.#invites.get(code);
    if (invite === undefined) {
      return;
    }
    this.#invites.delete(code);
    this.#touch(`invite:${code}`);
    this.#touch(`invites:${invite.community}`);
  }

  #addMember(communityId: string, userId: string): void {
    let members = this.#members.get(communityId);
    if (members === undefined) {
      members = new Set();
      this.#members.set(communityId, members);
    }
    if (members.has(userId)) {
      return;
    }
    members.add(userId);
    let memberOf = this.#memberOf.get(userId);
    if (memberOf === undefined) {
      memberOf = new Set();
      this.#memberOf.set(userId, memberOf);
    }
    memberOf.add(communityId);
    this.#touchMembers(communityId);
  }

  #removeMember(communityId: string, userId: string): void {
    const members = this.#members.get(communityId);
    if (!members?.delete(userId)) {
      return;
    }
    this.#memberOf.get(userId)?.delete(communityId);
    this.#touchMembers(communityId);
  }

  #replaceMembers(communityId: string, userIds: readonly string[]): void {
    for (const userId of Array.from(this.#members.get(communityId) ?? [])) {
      this.#removeMember(communityId, userId);
    }
    for (const userId of userIds) {
      this.#addMember(communityId, userId);
    }
  }
}

/** Splits a community's channels into those under each category and the top-level rest. */
/** Whether a channel is a DM or group DM. */
export function isDm(channel: Channel): boolean {
  return channel.ty === "dm" || channel.ty === "groupDm";
}

export function groupChannels(
  channels: readonly Channel[],
  categories: readonly Category[],
): { topLevel: Channel[]; byCategory: Map<string, Channel[]> } {
  const byCategory = new Map<string, Channel[]>(categories.map((c) => [c.id, []]));
  const topLevel: Channel[] = [];
  for (const channel of channels) {
    const group =
      channel.parentCategory == null ? undefined : byCategory.get(channel.parentCategory);
    if (group === undefined) {
      topLevel.push(channel);
    } else {
      group.push(channel);
    }
  }
  return { topLevel, byCategory };
}

export type { UserCommunity };
