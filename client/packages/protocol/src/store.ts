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

import { isDm } from "./channels";
import {
  isUnread,
  kindNotifies,
  levelOf,
  placeMentions,
  tagsMe,
  unreadPlaces,
} from "./notifyRules";
import {
  dmAccess,
  memberAccess,
  type CommunityPermissions,
  type OverrideGrant,
  type PermissionSet,
} from "./permissions";
import type {
  Attachment,
  HeldEntry,
  HeldMessage,
  BotCommands,
  ChannelMute,
  ChannelVoice,
  EmojiReactions,
  Icon,
  Included,
  KeptMessage,
  LinkedMessage,
  Listener,
  MessageWindow,
  MissingKind,
  NotificationLevel,
  NotificationSetting,
  Pin,
  PluginInfo,
  ReactionSummary,
  Reactions,
  ReadState,
  Topic,
  VoiceParticipantState,
} from "./storeTypes";
import type {
  Category,
  CategoryOverride,
  Channel,
  ChannelOverride,
  Community,
  CommunityBan,
  CommunityPlugin,
  CustomEmoji,
  DeploymentPermission,
  Invite,
  Message,
  MessageAnnotation,
  Poll,
  Role,
  ServerEvent,
  VoiceParticipant,
  VoiceRing,
  VoiceSession,
  User,
  UserAnnotation,
  UserCommunity,
} from "./generated/events";
import type { components } from "./generated/openapi";
import { identityOf } from "./identity";

type UserOnlineStatus = components["schemas"]["UserOnlineStatus"];

/**
 * The most messages a channel's window holds. Extending it past this at one end drops
 * messages, and their records, from the other end, so a long scroll through history never
 * holds more than this many messages in memory; what was dropped is read again on the way back.
 */
export const WINDOW_MAX_MESSAGES = 300;
/**
 * How many messages a window followed live may grow to before its oldest are dropped: twice
 * the cap, because the reader may be reading the oldest of a full window when a message
 * arrives, and dropping them from under the reader would move what they are reading; by the
 * time this many have arrived, they have long since left.
 */
export const LIVE_WINDOW_MAX_MESSAGES = WINDOW_MAX_MESSAGES * 2;

const NO_VOICE: ChannelVoice = { session: null, participants: [], rings: [] };

/** How many of an emoji's reactors a summary names, as the server's `SUMMARY_USERS`. */
export const REACTION_SUMMARY_USERS = 4;

const EMPTY_IDS: readonly string[] = [];
const NO_PERMISSIONS: PermissionSet = new Set();
/** How many arrivals `arrivedAt`, and departures `departedAt`, remember. */
const MAX_ARRIVALS = 200;

const EMPTY_OVERRIDES: readonly never[] = [];
const NO_ANNOTATIONS: readonly never[] = [];
const NO_PLUGINS: readonly PluginInfo[] = [];
const NO_TYPERS: readonly string[] = [];
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
  /** Records the server says do not exist, by kind, so no one waits on them. */
  readonly #missing = new Set<`${MissingKind}:${string}`>();
  readonly #communities = new Map<string, Community>();
  readonly #channels = new Map<string, Channel>();
  readonly #categories = new Map<string, Category>();
  readonly #messages = new Map<string, Message>();
  /** What the caller finds at each message another links to, by the linked message's id. */
  readonly #links = new Map<string, LinkedMessage>();
  /** The messages warnings are about, deleted or not, by id; read only with their warnings. */
  readonly #warned = new Map<string, KeptMessage>();
  readonly #attachments = new Map<string, Attachment>();
  /** The caller's held messages, by id, in the order they were held. */
  readonly #heldMessages = new Map<string, HeldEntry>();
  /**
   * Held messages already posted or dropped, which a `202` read after their event must not
   * bring back.
   */
  readonly #settledHeld = new Set<string>();
  readonly #icons = new Map<string, Icon>();
  readonly #voiceSessions = new Map<string, VoiceSession>();
  /** `session -> user -> participant`. */
  readonly #voiceParticipants = new Map<string, Map<string, VoiceParticipantState>>();
  /** Session id → rung user id → ring. */
  readonly #voiceRings = new Map<string, Map<string, VoiceRing>>();
  /** `community -> user ids`, the sample of members the server returns per community. */
  readonly #members = new Map<string, Set<string>>();
  /** `user -> community ids`, the reverse of `#members`, for invalidating member lists. */
  readonly #memberOf = new Map<string, Set<string>>();
  readonly #reactions = new Map<string, Reactions>();
  /** Every community's own emoji, by id. */
  readonly #customEmoji = new Map<string, CustomEmoji>();
  /** The standing bans of the communities whose bans a read has brought, by community then user. */
  readonly #bans = new Map<string, Map<string, CommunityBan>>();
  readonly #polls = new Map<string, Poll>();
  /** `poll -> option indices` the calling user voted for, for polls read with their votes. */
  readonly #myVotes = new Map<string, ReadonlySet<number>>();
  /** The options the caller wrote in, by poll, which an anonymous poll's record never says. */
  readonly #myWriteIns = new Map<string, ReadonlySet<number>>();
  readonly #readStates = new Map<string, ReadState>();
  readonly #mutes = new Map<string, ChannelMute>();
  /** The caller's notification settings: per channel, and per community. */
  readonly #channelLevels = new Map<string, NotificationLevel>();
  readonly #communityLevels = new Map<string, NotificationLevel>();
  readonly #collapsed = new Set<string>();
  /** The users the caller has blocked. */
  readonly #blocked = new Set<string>();
  /**
   * People the caller blocked on any deployment, by `identityOf`, this deployment's domain,
   * which names its own users' identities, and the caller's home's, whose records alone are
   * believed about where a user is from; `silenced` reads them.
   */
  #blockedIdentities: ReadonlySet<string> = new Set();
  #domain = "";
  #home: string | null = null;
  readonly #roles = new Map<string, Role>();
  /** Overrides by `channel/role` and `category/role`. */
  readonly #channelOverrides = new Map<string, ChannelOverride>();
  readonly #categoryOverrides = new Map<string, CategoryOverride>();
  /** `channel -> message -> pin`, for the channels whose pins have been loaded. */
  readonly #pins = new Map<string, Map<string, Pin>>();
  /** How many people are online in each channel whose count has been read. */
  readonly #channelOnline = new Map<string, number>();
  /**
   * Who is typing in each channel, `channel -> user -> until` (by `now()`), in the order they
   * began. Never read from the server: only the event stream's `ephemeral` frames tell of it.
   */
  readonly #typing = new Map<string, Map<string, number>>();
  /** When messages that arrived while the app was open came, for drawing them arriving. */
  readonly #arrivals = new Map<string, number>();
  /** When messages deleted while the app was open went, for drawing them going. */
  readonly #departures = new Map<string, number>();
  readonly #commands = new Map<string, readonly BotCommands[]>();
  /** What plugins say about each message, by message and then by annotation id. */
  readonly #annotations = new Map<string, Map<string, MessageAnnotation>>();
  /** Which message each held annotation is about, for the events that name only the id. */
  readonly #annotated = new Map<string, string>();
  /** What plugins say about each person whose annotations were read, by person and id. */
  readonly #userAnnotations = new Map<string, Map<string, UserAnnotation>>();
  #plugins: readonly PluginInfo[] = NO_PLUGINS;
  /** A community's use of each plugin, by community and plugin, for its plugins' managers. */
  readonly #communityPlugins = new Map<string, Map<string, CommunityPlugin>>();
  /** `community/user -> role ids` each member holds besides everyone's, as far as known. */
  readonly #memberRoles = new Map<string, readonly string[]>();
  /** `community/user -> nickname` of each member known to have one there. */
  readonly #nicknames = new Map<string, string>();
  /** What the caller may do across the deployment. */
  #deployment: ReadonlySet<DeploymentPermission> = new Set();
  /** How many report cases await review, for a reviewer, once read; and each change to them. */
  #openReports: number | undefined = undefined;
  #reportsChanges = 0;
  #emailChanges = 0;
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

  /**
   * A cached user of this deployment by username, ignoring case, as a person types one; those
   * of other deployments share names with it, so they are left out.
   */
  userNamed(name: string): User | undefined {
    const wanted = name.toLowerCase();
    for (const user of this.#users.values()) {
      if (user.homeDomain == null && user.name.toLowerCase() === wanted) {
        return user;
      }
    }
    return undefined;
  }

  /**
   * Whether the server said there is no such record of `kind`; topic `<kind>:<id>`, the one its
   * record would come on, so whatever waits for it hears the answer either way.
   */
  missing(kind: MissingKind, id: string): boolean {
    return this.#missing.has(`${kind}:${id}`);
  }

  /** Notes that the server knows no such record, which ends any wait for it. */
  markMissing(kind: MissingKind, id: string): void {
    const key = `${kind}:${id}` as const;
    if (!this.#missing.has(key)) {
      this.#missing.add(key);
      this.#touch(key);
    }
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

  /** Topic `link:<id>`: what the caller finds at a message another links to, once read. */
  linkedMessage(id: string): LinkedMessage | undefined {
    return this.#links.get(id);
  }

  /** Topic `warned:<id>`: a message a warning is about, as the warning shows it, once read. */
  warnedMessage(id: string): KeptMessage | undefined {
    return this.#warned.get(id);
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

  /** Topic `held:<channelId>`: the caller's messages there held for their previews. */
  heldMessages(channelId: string): readonly HeldEntry[] {
    return this.#memoized(`held:${channelId}`, () =>
      Array.from(this.#heldMessages.values()).filter(
        (entry) => entry.message.channelId === channelId,
      ),
    );
  }

  /** A held message's entry, when the store holds it. */
  heldMessage(id: string): HeldEntry | undefined {
    return this.#heldMessages.get(id);
  }

  /** Keeps a message the server held, unless its posting or dropping was heard of first. */
  putHeldMessage(message: HeldMessage): void {
    if (this.#settledHeld.has(message.id)) {
      return;
    }
    this.#heldMessages.set(message.id, { message, failure: null });
    this.#touch(`held:${message.channelId}`);
  }

  /**
   * Replaces the held messages with those the server holds, keeping those it dropped, which
   * only this app still shows.
   */
  replaceHeldMessages(messages: readonly HeldMessage[]): void {
    this.#batch(() => {
      for (const [id, entry] of this.#heldMessages) {
        if (entry.failure === null) {
          this.#heldMessages.delete(id);
          this.#touch(`held:${entry.message.channelId}`);
        }
      }
      for (const message of messages) {
        this.putHeldMessage(message);
      }
    });
  }

  /** Lets a dropped held message go, or one sent again. */
  forgetHeldMessage(id: string): void {
    const entry = this.#heldMessages.get(id);
    if (entry !== undefined) {
      this.#heldMessages.delete(id);
      this.#touch(`held:${entry.message.channelId}`);
    }
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
      const rings = Array.from(this.#voiceRings.get(session.id)?.values() ?? []);
      return { session, participants, rings };
    });
  }

  /**
   * Topic `rings`: the calls ringing the caller, in DMs and group DMs, including rings that have
   * run out by the clock (see `ChannelVoice.rings`).
   */
  myRings(): readonly VoiceRing[] {
    return this.#memoized("rings", () => {
      const me = this.#myUserId;
      return Array.from(this.#voiceRings.values()).flatMap((rings) =>
        Array.from(rings.values()).filter((ring) => ring.user === me),
      );
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

  /** Topic `emoji:<communityId>`: the community's own emoji, by name. */
  customEmoji(communityId: string): readonly CustomEmoji[] {
    return this.#memoized(`emoji:${communityId}`, () =>
      Array.from(this.#customEmoji.values())
        .filter((e) => e.community === communityId)
        .sort((a, b) => a.name.localeCompare(b.name)),
    );
  }

  /**
   * Topic `bans:<communityId>`: the community's standing bans, newest first, or `undefined`
   * before a read has brought them (`AspenSync.loadBans`); events then keep them.
   */
  bans(communityId: string): readonly CommunityBan[] | undefined {
    return this.#memoized(`bans:${communityId}`, () => {
      const held = this.#bans.get(communityId);
      return held === undefined
        ? undefined
        : Array.from(held.values()).sort((a, b) => b.bannedAt.localeCompare(a.bannedAt));
    });
  }

  /** Keeps a read's whole list of a community's standing bans. */
  replaceBans(communityId: string, bans: readonly CommunityBan[]): void {
    this.#batch(() => {
      this.#bans.set(communityId, new Map(bans.map((b) => [b.user, b])));
      this.#touch(`bans:${communityId}`);
    });
  }

  /** One custom emoji by id, under its community's `emoji:<communityId>` topic. */
  customEmojiById(id: string): CustomEmoji | undefined {
    return this.#customEmoji.get(id);
  }

  /**
   * Topic `roles:<communityId>`: the roles a member holds besides everyone's, or `undefined`
   * when no read has said.
   */
  memberRoles(communityId: string, userId: string): readonly string[] | undefined {
    return this.#memberRoles.get(`${communityId}/${userId}`);
  }

  /**
   * Topic `nicknames:<communityId>`: the name a member chose in the community, which it shows
   * in place of their display name, or `undefined` when they chose none or no read has said.
   */
  nickname(communityId: string, userId: string): string | undefined {
    return this.#nicknames.get(`${communityId}/${userId}`);
  }

  /** Topic `nicknames:<communityId>`: `user -> nickname` for every member known to have one. */
  nicknames(communityId: string): ReadonlyMap<string, string> {
    return this.#memoized(`nicknames:${communityId}`, () => {
      const prefix = `${communityId}/`;
      const found = new Map<string, string>();
      for (const [key, nickname] of this.#nicknames) {
        if (key.startsWith(prefix)) {
          found.set(key.slice(prefix.length), nickname);
        }
      }
      return found;
    });
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
      // Only the caller's moderation is known; anyone else is resolved as a member.
      const moderator = user === this.#myUserId && this.moderator;
      return memberAccess(holds, owner, moderator, () => this.roles(communityId));
    };
    if (userId !== undefined && userId !== this.#myUserId) {
      return compute(userId);
    }
    return this.#memoized(`access:${communityId}`, () => compute(this.#myUserId));
  }

  /**
   * What `userId` may do in a channel, given what they may do across its community: its
   * category's overrides and then its own, or its parent's for a thread. A category the store
   * does not hold is one whose own overrides hide it from the caller (the server sends no
   * other), so it counts as denying View channel, which the channel's own overrides may grant
   * again; what else it denies or allows is not known here, and the server decides.
   */
  permissionsIn(channelId: string, access: CommunityPermissions): PermissionSet {
    const channel = this.#channels.get(channelId);
    const governing =
      channel?.parentChannel != null ? this.#channels.get(channel.parentChannel) : channel;
    if (governing === undefined) {
      return NO_PERMISSIONS;
    }
    const category = governing.parentCategory;
    let categoryLayer: readonly OverrideGrant[] = EMPTY_OVERRIDES;
    if (category != null) {
      if (this.#categories.has(category)) {
        categoryLayer = this.categoryOverrides(category);
      } else {
        const everyone =
          governing.community == null
            ? undefined
            : this.roles(governing.community).find((r) => r.everyone);
        if (everyone !== undefined) {
          categoryLayer = [{ role: everyone.id, allow: [], deny: ["viewChannel"] }];
        }
      }
    }
    return access.inChannel(categoryLayer, this.channelOverrides(governing.id));
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
        // A moderator reading a DM they are not in may only look, and take things away.
        const parent =
          channel.parentChannel != null ? this.#channels.get(channel.parentChannel) : channel;
        const recipient =
          this.#myUserId !== null && (parent?.recipients ?? []).includes(this.#myUserId);
        // A block, or notices from the system account, leave a DM to be read and nothing more.
        const readOnly =
          recipient &&
          (this.blockedDmPeer(channelId) !== null || this.systemDmPeer(channelId) !== null);
        return dmAccess(recipient, readOnly, this.moderator);
      }
      const access = this.access(channel.community);
      return access === null ? NO_PERMISSIONS : this.permissionsIn(channelId, access);
    });
  }

  /**
   * Records the roles and nicknames of members a search or a read found, without making them
   * part of the community's member sample, which stays what the community read gave.
   */
  noteMemberships(memberships: readonly UserCommunity[]): void {
    this.#batch(() => {
      for (const membership of memberships) {
        this.#setMemberRoles(membership.community, membership.user, membership.roles);
        this.#setNickname(membership.community, membership.user, membership.nickname);
      }
    });
  }

  /**
   * When a message arrived, if it came while the app was open, by the stream or as the caller
   * sent it, rather than with a read of history; for drawing it arriving.
   */
  arrivedAt(messageId: string): number | undefined {
    return this.#arrivals.get(messageId);
  }

  /** When a message was deleted, if that happened while the app was open; for drawing it going. */
  departedAt(messageId: string): number | undefined {
    return this.#departures.get(messageId);
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

  /**
   * Topic `channel-online:<channelId>`: how many people who may view the channel are online,
   * or `undefined` until read.
   */
  channelOnline(channelId: string): number | undefined {
    return this.#channelOnline.get(channelId);
  }

  /** Installs a channel's online count as read. */
  setChannelOnline(channelId: string, online: number): void {
    if (this.#channelOnline.get(channelId) === online) {
      return;
    }
    this.#batch(() => {
      this.#channelOnline.set(channelId, online);
      this.#touch(`channel-online:${channelId}`);
    });
  }

  /** Installs a channel's pins as read, replacing what was held. */
  setPins(channelId: string, pins: readonly Pin[]): void {
    this.#batch(() => {
      this.#pins.set(channelId, new Map(pins.map((p) => [p.messageId, p])));
      this.#touch(`pins:${channelId}`);
    });
  }

  /**
   * Topic `commands:<channelId>`: each bot that can see the channel and has commands, with
   * them, or `undefined` until loaded or once something that may change them has happened.
   */
  /**
   * What the plugins the deployment runs say about a message, oldest first. An annotation of a
   * plugin it no longer runs is passed over.
   */
  annotations(messageId: string): readonly MessageAnnotation[] {
    return this.#memoized(`annotations:${messageId}`, () => {
      const held = this.#annotations.get(messageId);
      if (held === undefined) {
        return NO_ANNOTATIONS;
      }
      return Array.from(held.values())
        .filter((a) => this.plugin(a.plugin) !== undefined)
        .sort((a, b) => a.id.localeCompare(b.id));
    });
  }

  /** What the plugins the deployment runs say about a person; `undefined` until read. */
  userAnnotations(userId: string): readonly UserAnnotation[] | undefined {
    return this.#memoized(`userAnnotations:${userId}`, () => {
      const held = this.#userAnnotations.get(userId);
      if (held === undefined) {
        return undefined;
      }
      return Array.from(held.values())
        .filter((a) => this.plugin(a.plugin) !== undefined)
        .sort((a, b) => a.id.localeCompare(b.id));
    });
  }

  /** The plugins the deployment runs, in its order. */
  plugins(): readonly PluginInfo[] {
    return this.#plugins;
  }

  plugin(id: string): PluginInfo | undefined {
    return this.#plugins.find((p) => p.id === id);
  }

  /** A community's use of each plugin, for its plugins' managers; `undefined` until read. */
  communityPlugins(communityId: string): readonly CommunityPlugin[] | undefined {
    return this.#memoized(`communityPlugins:${communityId}`, () => {
      const held = this.#communityPlugins.get(communityId);
      return held === undefined ? undefined : Array.from(held.values());
    });
  }

  commands(channelId: string): readonly BotCommands[] | undefined {
    return this.#commands.get(channelId);
  }

  /** Installs a channel's commands as read. */
  setCommands(channelId: string, commands: readonly BotCommands[]): void {
    this.#batch(() => {
      this.#commands.set(channelId, commands);
      this.#touch(`commands:${channelId}`);
    });
  }

  /**
   * Drops every channel's commands, to be read again where they are shown. Which bots can see
   * a channel follows memberships, roles, and overrides, which the server resolves; a change
   * to any of them is rare enough that asking again beats resolving it here.
   */
  #forgetCommands(): void {
    for (const channelId of this.#commands.keys()) {
      this.#touch(`commands:${channelId}`);
    }
    this.#commands.clear();
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
    return isUnread(this.#readStates.get(channelId));
  }

  /**
   * Topic `read:<channelId>`: how many unread messages in the channel tag the caller. They
   * count in muted channels too, which is how a tag reaches someone through a mute.
   */
  mentions(channelId: string): number {
    return this.#readStates.get(channelId)?.mentions ?? 0;
  }

  /**
   * Topic `unread`: the unread tags of the caller across a community's channels, or across
   * their DMs with `UNREAD_DMS`.
   */
  placeMentions(place: string): number {
    return placeMentions(place, this.#readStates, this.#channels);
  }

  /**
   * Topic `notifications`: how much of a channel the caller wants to be told of (a thread
   * follows its parent): their setting for the channel, else for its community, else every
   * message of a DM and tags elsewhere. `own` is the channel's own setting, if it has one, and
   * `inherited` what the level would be without it.
   */
  notificationLevel(channelId: string): {
    level: NotificationLevel;
    own: NotificationLevel | null;
    inherited: NotificationLevel;
  } {
    const channel = this.#channels.get(channelId);
    const place =
      channel?.parentChannel != null ? this.#channels.get(channel.parentChannel) : channel;
    const own = this.#channelLevels.get(place?.id ?? channelId) ?? null;
    const community =
      place?.community == null ? undefined : this.#communityLevels.get(place.community);
    return levelOf(place, own, community);
  }

  /** Topic `notifications`: the caller's setting for a community, if they made one. */
  communityNotificationLevel(communityId: string): NotificationLevel | null {
    return this.#communityLevels.get(communityId) ?? null;
  }

  /**
   * Whether a message should notify the caller: someone else's, not someone they blocked here
   * or elsewhere, still unread, in a channel not muted, and at a level that asks for it.
   */
  notifies(message: Message): boolean {
    if (message.author === this.#myUserId || this.silenced(message.author)) {
      return false;
    }
    if (!kindNotifies(message)) {
      return false;
    }
    const channel = this.#channels.get(message.channelId);
    const place = channel?.parentChannel ?? message.channelId;
    // A mute that ran out is gone already: `AspenSync` ends it by the clock.
    if (this.#mutes.has(place)) {
      return false;
    }
    const read = this.#readStates.get(place);
    if (read !== undefined && message.id <= read.lastRead) {
      return false;
    }
    const { level } = this.notificationLevel(message.channelId);
    return level === "all" || (level === "tags" && this.mentionsMe(message));
  }

  /** Topic `mute:<channelId>`: the caller's mute of the channel while it lasts, if any. */
  mute(channelId: string): ChannelMute | undefined {
    return this.#mutes.get(channelId);
  }

  /**
   * Topic `block:<userId>`: whether the caller has blocked the user. Their messages are
   * collapsed, their reactions left out, and they are silenced and hidden in calls.
   */
  blocked(userId: string): boolean {
    return this.#blocked.has(userId);
  }

  /**
   * Topic `channelAccess:<channelId>`: the other person of a one-to-one DM (or of the DM a
   * thread is in) when the caller has blocked them, so neither may write there; otherwise
   * `null`. A block the other person made is theirs alone, and the server's refusal is the only
   * sign of it.
   */
  blockedDmPeer(channelId: string): string | null {
    const channel = this.#channels.get(channelId);
    const dm = channel?.parentChannel != null ? this.#channels.get(channel.parentChannel) : channel;
    if (dm?.ty !== "dm") {
      return null;
    }
    const other = dm.recipients.find((user) => user !== this.#myUserId);
    return other !== undefined && this.#blocked.has(other) ? other : null;
  }

  /**
   * Topic `channelAccess:<channelId>`: the system account, when it is the other person of a
   * one-to-one DM (or of the DM a thread is in), whose notices are read and not answered.
   */
  systemDmPeer(channelId: string): string | null {
    const channel = this.#channels.get(channelId);
    const dm = channel?.parentChannel != null ? this.#channels.get(channel.parentChannel) : channel;
    if (dm?.ty !== "dm") {
      return null;
    }
    const other = dm.recipients.find((user) => user !== this.#myUserId);
    return other !== undefined && this.#users.get(other)?.system === true ? other : null;
  }

  /**
   * Topic `bots`: the bots the caller owns, as far as the cache holds them, the oldest first.
   * `AspenSync.loadBots` reads them all.
   */
  ownedBots(): readonly User[] {
    return this.#memoized("bots", () =>
      Array.from(this.#users.values())
        .filter((user) => user.bot && user.botOwner != null && user.botOwner === this.#myUserId)
        .sort((a, b) => (a.id < b.id ? -1 : 1)),
    );
  }

  /** Forgets a user the caller deleted, such as their own bot, which no event may tell them of. */
  forgetUser(userId: string): void {
    this.#batch(() => {
      this.#removeUser(userId);
    });
  }

  /**
   * Topic `silenced`: whether the user is silenced and hidden in calls here, which a block made
   * here or on any other deployment the caller uses does (`setBlockedIdentities`). Only a block
   * made here does anything more.
   */
  silenced(userId: string): boolean {
    return this.#blocked.has(userId) || this.#blockedElsewhere(userId);
  }

  /**
   * Who the caller blocked on every deployment they use, by `identityOf`, with `domain`, the
   * name of this one, so its own users' identities can be told, and `home`, the caller's home's.
   */
  setBlockedIdentities(domain: string, home: string | null, identities: ReadonlySet<string>): void {
    const same =
      domain === this.#domain &&
      home === this.#home &&
      identities.size === this.#blockedIdentities.size &&
      [...identities].every((identity) => this.#blockedIdentities.has(identity));
    if (same) {
      return;
    }
    this.#batch(() => {
      this.#domain = domain;
      this.#home = home;
      this.#blockedIdentities = new Set(identities);
      this.#touch("silenced");
    });
  }

  #blockedElsewhere(userId: string): boolean {
    if (this.#blockedIdentities.size === 0) {
      return false;
    }
    const user = this.#users.get(userId);
    return this.#blockedIdentities.has(
      identityOf(user ?? { id: userId, homeDomain: null, homeId: null }, this.#domain, this.#home),
    );
  }

  /**
   * Topic `typing:<channelId>`: who is typing in the channel, in the order they began, leaving
   * out the caller and anyone they block on any deployment (`silenced`).
   */
  typers(channelId: string): readonly string[] {
    return this.#memoized(`typing:${channelId}`, () => {
      const typing = this.#typing.get(channelId);
      if (typing === undefined) {
        return NO_TYPERS;
      }
      return [...typing.keys()].filter((id) => id !== this.#myUserId && !this.silenced(id));
    });
  }

  /**
   * Notes that someone is typing in a channel until `until` (by `now()`), keeping their place
   * among those already typing, or, with `null`, that they stopped.
   */
  noteTyping(channelId: string, userId: string, until: number | null): void {
    this.#batch(() => {
      let typing = this.#typing.get(channelId);
      if (until === null) {
        if (typing?.delete(userId) !== true) {
          return;
        }
        if (typing.size === 0) {
          this.#typing.delete(channelId);
        }
      } else {
        if (typing === undefined) {
          typing = new Map();
          this.#typing.set(channelId, typing);
        }
        typing.set(userId, until);
      }
      this.#touch(`typing:${channelId}`);
    });
  }

  /**
   * Lets go of everyone whose typing ran out by `now`; returns when the next of those still
   * typing runs out, or `null` when nobody is.
   */
  expireTyping(now: number): number | null {
    let next: number | null = null;
    this.#batch(() => {
      for (const [channelId, typing] of this.#typing) {
        for (const [userId, until] of typing) {
          if (until <= now) {
            typing.delete(userId);
            this.#touch(`typing:${channelId}`);
          } else if (next === null || until < next) {
            next = until;
          }
        }
        if (typing.size === 0) {
          this.#typing.delete(channelId);
        }
      }
    });
    return next;
  }

  /** Forgets who is typing anywhere, once the connection that told of it is gone. */
  forgetTyping(): void {
    this.#batch(() => {
      for (const channelId of this.#typing.keys()) {
        this.#touch(`typing:${channelId}`);
      }
      this.#typing.clear();
    });
  }

  /** Topic `blocks`: everyone the caller has blocked. */
  blockedUsers(): readonly string[] {
    return this.#memoized("blocks", () => Array.from(this.#blocked));
  }

  /** The channels whose message windows are held. */
  heldWindows(): readonly string[] {
    return Array.from(this.#windows.keys());
  }

  /** Topic `admin`: what the caller may do across the deployment. */
  deploymentPermissions(): ReadonlySet<DeploymentPermission> {
    return this.#deployment;
  }

  /** Whether the caller moderates the deployment, and so reaches every community and DM. */
  get moderator(): boolean {
    return this.#deployment.has("moderateCommunities");
  }

  /** Topic `reports`: how many report cases await review, once a read or event has said. */
  get openReports(): number | undefined {
    return this.#openReports;
  }

  /** Topic `reports`: counts every change to what awaits review, for lists to read again. */
  get reportsChanges(): number {
    return this.#reportsChanges;
  }

  /**
   * Topic `email`: counts every change to the caller's email address or what they receive
   * there (`emailAccountChanged`), for the screen showing it to read it again.
   */
  get emailChanges(): number {
    return this.#emailChanges;
  }

  /** Records how many report cases are open, from a read or a `reportsChanged` event. */
  setOpenReports(open: number): void {
    this.#openReports = open;
    this.#reportsChanges += 1;
    this.#touch("reports");
  }

  /**
   * Records what the caller may do across the deployment, as the server says. Moderating it
   * changes what they may do everywhere.
   */
  setDeploymentPermissions(permissions: readonly DeploymentPermission[]): void {
    this.#batch(() => {
      const wasModerator = this.moderator;
      this.#deployment = new Set(permissions);
      this.#touch("admin");
      if (wasModerator !== this.moderator) {
        for (const id of this.#communities.keys()) {
          this.#accessChanged(id);
        }
        for (const channel of this.#channels.values()) {
          this.#touch(`channelAccess:${channel.id}`);
        }
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
    return this.#memoized("unread", () =>
      unreadPlaces(this.#readStates, this.#channels, this.#mutes),
    );
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
  /**
   * How recently a DM was active, as the id of a message in it: the newest seen arrive, or, for
   * one only listed so far, the newer of its newest message by someone else and where the
   * caller read up to, which their own posts move. Message ids are UUIDv7s, ordered by time, so
   * these compare across deployments too, which is what the one DM list of every deployment
   * sorts by. `undefined` for a DM with nothing in it.
   */
  dmActivity(channelId: string): string | undefined {
    const state = this.#readStates.get(channelId);
    const candidates = [
      this.#dmActivity.get(channelId),
      state?.lastMessage ?? undefined,
      state?.lastRead ?? undefined,
    ].filter((id): id is string => id !== undefined && id !== "");
    return candidates.reduce<string | undefined>(
      (newest, id) => (newest === undefined || id > newest ? id : newest),
      undefined,
    );
  }

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

      // Pins and bans are kept current by events, some of which may have been missed; they are
      // read afresh when next shown.
      for (const id of this.#pins.keys()) {
        this.#touch(`pins:${id}`);
      }
      this.#pins.clear();
      for (const id of this.#bans.keys()) {
        this.#touch(`bans:${id}`);
      }
      this.#bans.clear();
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
      for (const id of this.#links.keys()) {
        this.#touch(`link:${id}`);
      }
      this.#links.clear();
      for (const id of this.#warned.keys()) {
        this.#touch(`warned:${id}`);
      }
      this.#warned.clear();
      for (const messageId of this.#reactions.keys()) {
        this.#touch(`reactions:${messageId}`);
      }
      this.#reactions.clear();
      for (const messageId of Array.from(this.#annotations.keys())) {
        this.#clearAnnotations(messageId);
      }
      for (const userId of this.#userAnnotations.keys()) {
        this.#touch(`userAnnotations:${userId}`);
      }
      this.#userAnnotations.clear();
      for (const communityId of this.#communityPlugins.keys()) {
        this.#touch(`communityPlugins:${communityId}`);
      }
      this.#communityPlugins.clear();
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
      this.#voiceRings.clear();
      this.#touch("rings");
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
      for (const link of included.linkedMessages ?? []) {
        this.#links.set(link.id, link);
        this.#touch(`link:${link.id}`);
      }
      for (const kept of included.warnedMessages ?? []) {
        this.#warned.set(kept.message.id, kept);
        this.#touch(`warned:${kept.message.id}`);
      }
      for (const category of included.categories ?? []) {
        this.#putCategory(category);
      }
      for (const attachment of included.attachments ?? []) {
        this.#putAttachment(attachment);
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
      for (const setting of included.notificationSettings ?? []) {
        this.#putNotificationSetting(
          setting.community ?? null,
          setting.channel ?? null,
          setting.level,
        );
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
      if (included.customEmoji !== undefined) {
        this.#replaceCustomEmoji(included.customEmoji);
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
      // Rings likewise come whole for the sessions they come with.
      if (included.voiceRings !== undefined) {
        for (const session of included.voiceSessions ?? []) {
          this.#voiceRings.set(session.id, new Map());
        }
        for (const ring of included.voiceRings) {
          this.#putVoiceRing(ring);
        }
        this.#touch("rings");
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
          this.#setNickname(membership.community, membership.user, membership.nickname);
          // Only the caller's own membership says where it sits in their list.
          if (membership.user === this.#myUserId && membership.sortIndex != null) {
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
        this.#noteArrival(message.id);
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
  /** Replaces every notification setting held with `settings`, the complete list a bootstrap read. */
  replaceNotificationSettings(settings: readonly NotificationSetting[]): void {
    this.#batch(() => {
      this.#channelLevels.clear();
      this.#communityLevels.clear();
      for (const setting of settings) {
        this.#putNotificationSetting(
          setting.community ?? null,
          setting.channel ?? null,
          setting.level,
        );
      }
      this.#touch("notifications");
    });
  }

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

  /**
   * Replaces everyone blocked with `userIds`, the complete list a bootstrap read, so a block
   * lifted while the stream was away does not linger.
   */
  replaceBlocks(userIds: readonly string[]): void {
    this.#batch(() => {
      const listed = new Set(userIds);
      for (const userId of Array.from(this.#blocked)) {
        if (!listed.has(userId)) {
          this.#setBlocked(userId, false);
        }
      }
      for (const userId of userIds) {
        this.#setBlocked(userId, true);
      }
    });
  }

  /** Records that the caller blocked or unblocked someone. Returns whether that changed. */
  setBlocked(userId: string, blocked: boolean): boolean {
    let changed = false;
    this.#batch(() => {
      changed = this.#setBlocked(userId, blocked);
    });
    return changed;
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

  /**
   * Stores a read state the server sent for one channel, replacing what was held but for a
   * position already further on, since a position only moves forward and this device may have
   * read on before telling the server.
   */
  putReadState(state: ReadState): void {
    this.#batch(() => {
      const held = this.#readStates.get(state.channel);
      this.#putReadState(
        held !== undefined && held.lastRead > state.lastRead
          ? { ...state, lastRead: held.lastRead }
          : state,
      );
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
      this.#heldMessages.clear();
      this.#settledHeld.clear();
      this.#icons.clear();
      this.#voiceSessions.clear();
      this.#voiceParticipants.clear();
      this.#voiceRings.clear();
      this.#touch("rings");
      this.#members.clear();
      this.#memberOf.clear();
      this.#reactions.clear();
      this.#annotations.clear();
      this.#annotated.clear();
      this.#userAnnotations.clear();
      this.#communityPlugins.clear();
      this.#plugins = NO_PLUGINS;
      this.#polls.clear();
      this.#myVotes.clear();
      this.#myWriteIns.clear();
      this.#readStates.clear();
      this.#mutes.clear();
      this.#collapsed.clear();
      this.#blocked.clear();
      this.#roles.clear();
      this.#customEmoji.clear();
      this.#bans.clear();
      this.#pins.clear();
      this.#channelOnline.clear();
      this.#forgetCommands();
      this.#channelOverrides.clear();
      this.#categoryOverrides.clear();
      this.#memberRoles.clear();
      this.#nicknames.clear();
      this.#deployment = new Set();
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
        case "communityBan": {
          // Only a community whose bans were read is followed; the rest are read when shown.
          const held = this.#bans.get(event.community);
          if (held !== undefined) {
            if (event.type === "create") {
              held.set(event.user, created(event));
            } else {
              held.delete(event.user);
            }
            this.#touch(`bans:${event.community}`);
          }
          break;
        }
        case "customEmoji":
          if (event.type === "create") {
            this.#putCustomEmoji(created(event));
          } else if (event.type === "update") {
            const emoji = this.#customEmoji.get(event.id);
            if (emoji !== undefined) {
              this.#putCustomEmoji(mergePatch(emoji, event));
            }
          } else {
            this.#removeCustomEmoji(event.id);
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
            this.#setNickname(event.community, event.user, event.nickname);
            if (event.user === this.#myUserId) {
              this.#myCommunities.add(event.community);
              if (event.sortIndex != null) {
                this.#setMyOrder(event.community, event.sortIndex);
              }
              this.#touch("communities");
            }
          } else if (event.type === "update") {
            if (event.user === this.#myUserId && event.sortIndex != null) {
              this.#setMyOrder(event.community, event.sortIndex);
            }
            if (event.roles != null) {
              this.#setMemberRoles(event.community, event.user, event.roles);
            }
            if (event.nickname !== undefined) {
              this.#setNickname(event.community, event.user, event.nickname);
            }
          } else {
            this.#removeMember(event.community, event.user);
            this.#setMemberRoles(event.community, event.user, undefined);
            this.#setNickname(event.community, event.user, undefined);
            // Leaving, being removed, or being banned takes the whole community away: nothing
            // of it reaches the caller any more, so nothing held of it stays.
            if (event.user === this.#myUserId) {
              this.#removeCommunity(event.community);
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
                // A move into another category may hide it: the server tells those who could
                // view it before the move, and then nothing more.
                if (
                  event.parentCategory !== undefined &&
                  updated.community != null &&
                  this.#decided(updated.community) &&
                  !this.channelAccess(updated.id).has("viewChannel")
                ) {
                  this.#removeChannel(updated.id);
                }
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
            if (!this.#messages.has(message.id)) {
              this.#noteArrival(message.id);
            }
            this.#putMessage(message);
            this.#appendToWindow(message);
            this.#noteDmActivity(message.channelId, message.id);
            this.#noteNewMessage(message);
            // Whoever posted has stopped typing it, whether or not they said so first.
            this.noteTyping(message.channelId, message.author, null);
          } else if (event.type === "update") {
            const message = this.#messages.get(event.id);
            if (message !== undefined) {
              this.#putMessage(mergePatch(message, event));
            }
          } else {
            if (this.#messages.has(event.id)) {
              this.#departures.set(event.id, this.#now());
              this.#trimArrivals(this.#departures);
            }
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
        case "voiceRing":
          if (event.type === "create") {
            this.#putVoiceRing(created(event));
          } else {
            this.#removeVoiceRing(event.session, event.user);
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
        case "deploymentAccessChanged":
          this.setDeploymentPermissions(event.permissions);
          break;
        case "reportsChanged":
          this.setOpenReports(event.open);
          break;
        case "emailAccountChanged":
          // The address stays out of the stream; whoever shows it reads it again.
          this.#emailChanges += 1;
          this.#touch("email");
          break;
        case "accountBanned":
          // The stream closes after this, and the sign-in with it; nothing here to keep.
          break;
        case "userBlockChanged":
          this.#setBlocked(event.user, event.blocked);
          break;
        case "categoryCollapseChanged":
          this.#setCollapsed(event.category, event.collapsed);
          break;
        case "notificationSettingChanged":
          this.#putNotificationSetting(
            event.community ?? null,
            event.channel ?? null,
            event.level ?? null,
          );
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
        case "botCommandsChanged":
        case "botCommandInvoked":
          // A bot's own stream hears of invocations; its commands follow below.
          break;
        case "messageAnnotation":
          if (event.type === "create") {
            this.#putAnnotation(created(event));
          } else {
            const messageId = this.#annotated.get(event.id);
            const held = messageId === undefined ? undefined : this.#annotations.get(messageId);
            const current = held?.get(event.id);
            if (messageId !== undefined && held !== undefined && current !== undefined) {
              if (event.type === "update") {
                held.set(event.id, mergePatch(current, event));
              } else {
                held.delete(event.id);
                this.#annotated.delete(event.id);
              }
              this.#touch(`annotations:${messageId}`);
            }
          }
          break;
        case "userAnnotation":
          if (event.type === "create") {
            const annotation = created(event);
            const held = this.#userAnnotations.get(annotation.user);
            if (held !== undefined) {
              held.set(annotation.id, annotation);
              this.#touch(`userAnnotations:${annotation.user}`);
            }
          } else {
            for (const [userId, held] of this.#userAnnotations) {
              const current = held.get(event.id);
              if (current === undefined) {
                continue;
              }
              if (event.type === "update") {
                held.set(event.id, mergePatch(current, event));
              } else {
                held.delete(event.id);
              }
              this.#touch(`userAnnotations:${userId}`);
            }
          }
          break;
        case "communityPlugin": {
          const held = this.#communityPlugins.get(event.community);
          if (held === undefined) {
            break;
          }
          if (event.type === "create") {
            held.set(event.plugin, created(event));
          } else if (event.type === "update") {
            const current = held.get(event.plugin);
            if (current !== undefined) {
              held.set(event.plugin, mergePatch(current, event));
            }
          } else {
            held.delete(event.plugin);
          }
          this.#touch(`communityPlugins:${event.community}`);
          break;
        }
        case "pluginEvent":
          // A plugin's own events are for views of its own, which this client does not run.
          break;
        case "attachmentPreviewed": {
          const attachment = this.#attachments.get(event.attachment);
          if (attachment !== undefined) {
            this.#attachments.set(attachment.id, { ...attachment, preview: event.preview });
            this.#touch(`attachment:${attachment.id}`);
          }
          break;
        }
        case "heldMessagePosted":
          // The message itself arrives by its own event.
          this.#settledHeld.add(event.held);
          this.forgetHeldMessage(event.held);
          break;
        case "heldMessageFailed": {
          this.#settledHeld.add(event.held);
          const entry = this.#heldMessages.get(event.held);
          if (entry !== undefined) {
            this.#heldMessages.set(event.held, { ...entry, failure: event.detail });
            this.#touch(`held:${event.channel}`);
          }
          break;
        }
      }
      if (this.#commands.size > 0 && this.#changesCommands(event)) {
        this.#forgetCommands();
      }
    });
  }

  /** Whether `event` may change which commands a channel offers. */
  #changesCommands(event: ServerEvent): boolean {
    switch (event.serverEvent) {
      case "botCommandsChanged":
      case "role":
      case "channelOverride":
      case "categoryOverride":
        return true;
      case "userCommunity":
        return (
          (event.type !== "update" || event.roles != null) &&
          this.#users.get(event.user)?.bot === true
        );
      case "channel":
        return event.type === "update" && event.recipients != null;
      default:
        return false;
    }
  }

  // ---------------------------------------------------------------------------------------
  // Internals

  /**
   * Keeps an attachment record. A preview is never taken away once made, so a record read
   * before its preview was made keeps the preview `attachmentPreviewed` brought meanwhile.
   */
  #putAttachment(attachment: Attachment): void {
    const preview = attachment.preview ?? this.#attachments.get(attachment.id)?.preview;
    this.#attachments.set(attachment.id, preview == null ? attachment : { ...attachment, preview });
    this.#touch(`attachment:${attachment.id}`);
  }

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
    // Who is shown typing leaves out whoever is silenced.
    if (topic === "silenced") {
      for (const channelId of this.#typing.keys()) {
        this.#touch(`typing:${channelId}`);
      }
    }
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
    const previous = this.#users.get(user.id);
    this.#users.set(user.id, user);
    this.#touch(`user:${user.id}`);
    // A record can say who someone is elsewhere, which may be someone blocked there.
    if (this.#blockedElsewhere(user.id)) {
      this.#touch("silenced");
    }
    if (user.bot || previous?.bot === true) {
      this.#touch("bots");
    }
    // What the caller may do in a DM follows whether its other person is the system account.
    if (user.system && previous?.system !== true) {
      for (const channel of this.#channels.values()) {
        if (channel.ty === "dm" && channel.recipients.includes(user.id)) {
          this.#touch(`channelAccess:${channel.id}`);
        }
      }
    }
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
    const user = this.#users.get(id);
    if (!this.#users.delete(id)) {
      return;
    }
    this.#touch(`user:${id}`);
    if (user?.bot === true) {
      this.#touch("bots");
    }
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
    for (const key of Array.from(this.#nicknames.keys())) {
      if (key.startsWith(`${id}/`)) {
        this.#nicknames.delete(key);
      }
    }
    this.#touch(`roles:${id}`);
    this.#touch(`nicknames:${id}`);
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

  #putCustomEmoji(emoji: CustomEmoji): void {
    this.#customEmoji.set(emoji.id, emoji);
    this.#touch(`emoji:${emoji.community}`);
  }

  /**
   * Removes an emoji, and the reactions made with it, which the server's cascade took and
   * announced no further; a message's text still naming it renders as unknown.
   */
  #removeCustomEmoji(id: string): void {
    const emoji = this.#customEmoji.get(id);
    if (emoji === undefined) {
      return;
    }
    this.#customEmoji.delete(id);
    this.#touch(`emoji:${emoji.community}`);
    const key = `<:${id}>`;
    for (const [messageId, reactions] of Array.from(this.#reactions)) {
      if (reactions.has(key)) {
        const next = new Map(reactions);
        next.delete(key);
        this.#reactions.set(messageId, next);
        this.#touch(`reactions:${messageId}`);
      }
    }
  }

  /** A read's whole list of its communities' emoji replaces what was held for them. */
  #replaceCustomEmoji(emoji: readonly CustomEmoji[]): void {
    const communities = new Set(emoji.map((e) => e.community));
    for (const held of Array.from(this.#customEmoji.values())) {
      if (communities.has(held.community)) {
        this.#customEmoji.delete(held.id);
      }
    }
    for (const e of emoji) {
      this.#customEmoji.set(e.id, e);
    }
    for (const community of communities) {
      this.#touch(`emoji:${community}`);
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

  #setNickname(communityId: string, userId: string, nickname: string | null | undefined): void {
    const key = `${communityId}/${userId}`;
    if ((nickname ?? undefined) === this.#nicknames.get(key)) {
      return;
    }
    if (nickname == null) {
      this.#nicknames.delete(key);
    } else {
      this.#nicknames.set(key, nickname);
    }
    this.#touch(`nicknames:${communityId}`);
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
   * depends on it is recomputed, and the channels they may no longer view, and the invites of
   * others they may no longer manage, are let go of, as the server stops sending anything about
   * them.
   */
  #accessChanged(communityId: string): void {
    this.#touch(`roles:${communityId}`);
    this.#touch(`access:${communityId}`);
    const channels = Array.from(this.#channels.values()).filter((c) => c.community === communityId);
    for (const channel of channels) {
      this.#touch(`channelAccess:${channel.id}`);
    }
    if (this.#decided(communityId)) {
      for (const channel of channels) {
        if (!this.channelAccess(channel.id).has("viewChannel")) {
          this.#removeChannel(channel.id);
        }
      }
      // A category is the caller's to know while its own overrides leave them View channel
      // there; the server sends nothing more of one they lose, so it is let go here. Its
      // channels they may still view stay, under a category they do not know.
      const access = this.access(communityId);
      for (const category of Array.from(this.#categories.values())) {
        if (
          category.community === communityId &&
          access !== null &&
          !access.inChannel(this.categoryOverrides(category.id), EMPTY_OVERRIDES).has("viewChannel")
        ) {
          this.#forgetCategory(category.id);
        }
      }
      // Without Manage invites, only the caller's own invites are theirs to see.
      if (this.access(communityId)?.has("manageInvites") !== true) {
        for (const invite of Array.from(this.#invites.values())) {
          if (invite.community === communityId && invite.createdBy !== this.#myUserId) {
            this.#removeInvite(invite.code);
          }
        }
      }
      // Without Ban members no ban events arrive, so a list held now would go stale; it is read
      // afresh if the permission comes back.
      if (this.access(communityId)?.has("banMembers") !== true && this.#bans.delete(communityId)) {
        this.#touch(`bans:${communityId}`);
      }
    }
  }

  /**
   * Whether what the caller may do in a community can be decided: both its roles and the
   * caller's own are known (or the caller moderates the deployment).
   */
  #decided(communityId: string): boolean {
    return (
      (this.moderator || this.#memberRoles.has(`${communityId}/${this.#myUserId ?? ""}`)) &&
      this.roles(communityId).some((r) => r.everyone)
    );
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

  #putNotificationSetting(
    community: string | null,
    channel: string | null,
    level: NotificationLevel | null,
  ): void {
    const levels = channel === null ? this.#communityLevels : this.#channelLevels;
    const id = channel ?? community;
    if (id === null) {
      return;
    }
    if (level === null) {
      levels.delete(id);
    } else {
      levels.set(id, level);
    }
    this.#touch("notifications");
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

  #setBlocked(userId: string, blocked: boolean): boolean {
    if (this.#blocked.has(userId) === blocked) {
      return false;
    }
    if (blocked) {
      this.#blocked.add(userId);
    } else {
      this.#blocked.delete(userId);
    }
    this.#touch(`block:${userId}`);
    this.#touch("blocks");
    this.#touch("silenced");
    // What the caller may do in a one-to-one DM with them, and in its threads, changes too.
    for (const channel of this.#channels.values()) {
      const dm =
        channel.parentChannel != null ? this.#channels.get(channel.parentChannel) : channel;
      if (dm?.ty === "dm" && dm.recipients.includes(userId)) {
        this.#touch(`channelAccess:${channel.id}`);
      }
    }
    return true;
  }

  #putReadState(given: ReadState): void {
    // Tags are among the unread, so a channel read to its newest message holds none.
    const read = given.lastMessage == null || given.lastMessage <= given.lastRead;
    const state = given.mentions > 0 && read ? { ...given, mentions: 0 } : given;
    this.#readStates.set(state.channel, state);
    this.#touch(`read:${state.channel}`);
    this.#touch("unread");
  }

  /**
   * Keeps read states current as messages arrive: someone else's message is the channel's
   * newest, unless the caller blocked them, and the caller's own is read, as the server
   * records it. A channel with no read
   * state yet, one made since the caller's channels were last read, is unread from its start.
   * Threads keep no read state.
   */
  /** Notes that a message arrived now, forgetting the oldest past `MAX_ARRIVALS`. */
  #noteArrival(id: string): void {
    this.#arrivals.set(id, this.#now());
    this.#trimArrivals(this.#arrivals);
  }

  /** Forgets the oldest of `times` past `MAX_ARRIVALS`. */
  #trimArrivals(times: Map<string, number>): void {
    if (times.size > MAX_ARRIVALS) {
      const oldest = times.keys().next().value;
      if (oldest !== undefined) {
        times.delete(oldest);
      }
    }
  }

  #noteNewMessage(message: Message): void {
    const channel = this.#channels.get(message.channelId);
    if (channel === undefined || channel.ty === "thread") {
      return;
    }
    const state = this.#readStates.get(message.channelId) ?? {
      channel: message.channelId,
      lastRead: "",
      lastMessage: null,
      mentions: 0,
    };
    if (message.author === this.#myUserId) {
      if (message.id > state.lastRead) {
        this.#putReadState({ ...state, lastRead: message.id });
      }
    } else if (this.#blocked.has(message.author)) {
      // Someone the caller blocked never makes a channel unread for them.
    } else {
      const newest =
        state.lastMessage == null || message.id > state.lastMessage
          ? message.id
          : state.lastMessage;
      const tagged = message.id > state.lastRead && this.mentionsMe(message);
      if (newest !== state.lastMessage || tagged) {
        this.#putReadState({
          ...state,
          lastMessage: newest,
          mentions: state.mentions + (tagged ? 1 : 0),
        });
      }
    }
  }

  /**
   * Whether a message tags the caller, as the server decided its tags: by name, through a
   * role they hold in its community, or as everyone.
   */
  mentionsMe(message: Message): boolean {
    const me = this.#myUserId;
    if (me === null) {
      return false;
    }
    const community = this.#channels.get(message.channelId)?.community;
    const held = community == null ? undefined : this.#memberRoles.get(`${community}/${me}`);
    return tagsMe(message.mentions, me, held);
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
    // No pin events reach a channel the caller no longer has, so its pins are read afresh if it
    // comes back.
    if (this.#pins.delete(id)) {
      this.#touch(`pins:${id}`);
    }
    // What links into it showed is no longer the caller's to see.
    for (const link of Array.from(this.#links.values())) {
      if (link.state === "available" && this.#messages.get(link.id)?.channelId === id) {
        this.#links.set(link.id, { ...link, state: "unavailable" });
        this.#touch(`link:${link.id}`);
        this.#removeMessage(link.id);
      }
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

  /** Removes a deleted category, whose overrides went with it. */
  #removeCategory(id: string): void {
    if (!this.#forgetCategory(id)) {
      return;
    }
    // Channels filed under the category become top-level rather than disappearing.
    for (const channel of this.#channels.values()) {
      if (channel.parentCategory === id) {
        this.#putChannel({ ...channel, parentCategory: null });
      }
    }
  }

  /**
   * Lets go of a category and its overrides, leaving the channels filed under it as they are.
   * Returns whether it was held.
   */
  #forgetCategory(id: string): boolean {
    const category = this.#categories.get(id);
    if (category === undefined) {
      return false;
    }
    this.#categories.delete(id);
    for (const [key, o] of Array.from(this.#categoryOverrides)) {
      if (o.category === id) {
        this.#categoryOverrides.delete(key);
      }
    }
    this.#touch(`overrides:${id}`);
    this.#touch(`category:${id}`);
    this.#touch(`categories:${category.community}`);
    for (const channel of this.#channels.values()) {
      if (channel.parentCategory === id) {
        this.#touch(`channelAccess:${channel.id}`);
      }
    }
    return true;
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
    if (this.#voiceRings.delete(id)) {
      this.#touch("rings");
    }
    this.#touch(`voice:${session.channel}`);
  }

  #putVoiceRing(ring: VoiceRing): void {
    let rings = this.#voiceRings.get(ring.session);
    if (rings === undefined) {
      rings = new Map();
      this.#voiceRings.set(ring.session, rings);
    }
    rings.set(ring.user, ring);
    this.#touch(`voice:${ring.channel}`);
    this.#touch("rings");
  }

  #removeVoiceRing(session: string, user: string): void {
    const ring = this.#voiceRings.get(session)?.get(user);
    if (ring === undefined) {
      return;
    }
    this.#voiceRings.get(session)?.delete(user);
    this.#touch(`voice:${ring.channel}`);
    this.#touch("rings");
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
  /**
   * Installs what plugins say about `messageIds`, as a read that asked for annotations
   * returned them: every message it read has exactly the annotations listed for it.
   */
  setAnnotations(messageIds: readonly string[], annotations: readonly MessageAnnotation[]): void {
    this.#batch(() => {
      for (const id of messageIds) {
        this.#clearAnnotations(id);
      }
      for (const annotation of annotations) {
        this.#putAnnotation(annotation);
      }
    });
  }

  /** Installs what plugins say about a person, as `GET /users/{user}/annotations` answered. */
  setUserAnnotations(userId: string, annotations: readonly UserAnnotation[]): void {
    this.#batch(() => {
      this.#userAnnotations.set(userId, new Map(annotations.map((a) => [a.id, a])));
      this.#touch(`userAnnotations:${userId}`);
    });
  }

  /** Installs the plugins the deployment runs, as `GET /plugins` answered. */
  setPlugins(plugins: readonly PluginInfo[]): void {
    this.#batch(() => {
      this.#plugins = plugins;
      this.#touch("plugins");
      // Annotations are shown only for plugins the deployment runs.
      for (const id of this.#annotations.keys()) {
        this.#touch(`annotations:${id}`);
      }
      for (const id of this.#userAnnotations.keys()) {
        this.#touch(`userAnnotations:${id}`);
      }
    });
  }

  /** Installs a community's use of each plugin, as its plugins' managers read it. */
  setCommunityPlugins(communityId: string, plugins: readonly CommunityPlugin[]): void {
    this.#batch(() => {
      this.#communityPlugins.set(communityId, new Map(plugins.map((p) => [p.plugin, p])));
      this.#touch(`communityPlugins:${communityId}`);
    });
  }

  /** Stores one community's use of one plugin, as a write answered it. */
  putCommunityPlugin(record: CommunityPlugin): void {
    this.#batch(() => {
      let held = this.#communityPlugins.get(record.community);
      if (held === undefined) {
        held = new Map();
        this.#communityPlugins.set(record.community, held);
      }
      held.set(record.plugin, record);
      this.#touch(`communityPlugins:${record.community}`);
    });
  }

  #putAnnotation(annotation: MessageAnnotation): void {
    let held = this.#annotations.get(annotation.message);
    if (held === undefined) {
      held = new Map();
      this.#annotations.set(annotation.message, held);
    }
    held.set(annotation.id, annotation);
    this.#annotated.set(annotation.id, annotation.message);
    this.#touch(`annotations:${annotation.message}`);
  }

  #clearAnnotations(messageId: string): void {
    const held = this.#annotations.get(messageId);
    if (held === undefined) {
      return;
    }
    for (const id of held.keys()) {
      this.#annotated.delete(id);
    }
    this.#annotations.delete(messageId);
    this.#touch(`annotations:${messageId}`);
  }

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
    // The server leaves out the reactions of those the caller blocked, and so does the stream.
    if (this.#blocked.has(userId)) {
      return;
    }
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
    // A link to it now finds it deleted.
    const link = this.#links.get(id);
    if (link?.state === "available") {
      this.#links.set(id, { ...link, state: "deleted" });
      this.#touch(`link:${id}`);
    }
    const message = this.#messages.get(id);
    if (message === undefined) {
      return;
    }
    this.#messages.delete(id);
    this.#touch(`message:${id}`);
    if (this.#reactions.delete(id)) {
      this.#touch(`reactions:${id}`);
    }
    this.#clearAnnotations(id);
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
    // A window followed live for long enough would otherwise grow without bound; the oldest go
    // only once it is well past the cap (`LIVE_WINDOW_MAX_MESSAGES`), never from under a reader.
    if (ids.length > LIVE_WINDOW_MAX_MESSAGES) {
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
    this.#clearAnnotations(id);
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

export type { UserCommunity };
