/**
 * The shapes of what `RecordStore` holds and hands out: the server's records it names, and the
 * client's own views of them (message windows, calls, reaction summaries, topics).
 */

import type { VoiceRing, VoiceSession } from "./generated/events";
import type { components } from "./generated/openapi";

export type Attachment = components["schemas"]["Attachment"];
export type Icon = components["schemas"]["Icon"];
/** A message held by the server while a preview of one of its attachments is made. */
export type HeldMessage = components["schemas"]["HeldMessage"];

/**
 * A held message of the caller's as their app shows it until it is posted: waiting, or dropped
 * with `failure` saying why, kept so they can send it again or let it go.
 */
export interface HeldEntry {
  message: HeldMessage;
  failure: string | null;
}
export type Included = components["schemas"]["Included"];
/**
 * What the caller finds at a message another links to: `available` (its record is held as any
 * message is), `deleted`, or `unavailable` (no such message, or one they may not read), and for
 * an available one in a community, that community.
 */
export type LinkedMessage = components["schemas"]["LinkedMessage"];
/** A message as a warning shows it, or a review reads it: deleted ones too. */
export type KeptMessage = components["schemas"]["ReviewedMessage"];
export type PollVote = components["schemas"]["PollVote"];
/**
 * How far the caller has read a channel. `lastRead` is a position among the channel's message
 * ids, whose lexical order is chronological, so the channel is unread while `lastMessage` sorts
 * after it. An empty `lastRead` is before every message.
 */
export type ReadState = components["schemas"]["ReadState"];
/** A channel the caller has muted; `until` is `null` for a mute that lasts until lifted. */
export type ChannelMute = components["schemas"]["ChannelMute"];
export type NotificationLevel = components["schemas"]["NotificationLevel"];
export type NotificationSetting = components["schemas"]["NotificationSetting"];
/** Who a message tags, as far as its author was allowed to; the tags that count. */
export type Mentions = components["schemas"]["Mentions"];
/** Someone the caller has blocked, and since when. */
export type UserBlock = components["schemas"]["UserBlock"];
/** A pinned message: which, when it was pinned, and its place among the channel's pins. */
export type Pin = components["schemas"]["Pin"];
/** A bot and the commands it answers, as a channel offers them. */
export type BotCommands = components["schemas"]["BotCommands"];
export type Command = components["schemas"]["Command"];
export type CommandParameter = components["schemas"]["Parameter"];
export type ParameterType = components["schemas"]["ParameterType"];
/** A command as sent: which bot's, its name, and its arguments as the server reads them. */
export type Invocation = components["schemas"]["Invocation"];
/**
 * A plugin the deployment runs, as every client knows it: what it is, and its text in the
 * reader's language, from which its annotations, `alteredBy`, and settings forms are drawn.
 */
export type PluginInfo = components["schemas"]["PluginInfo"];
/** One of a plugin's settings, as a form draws it. */
export type SettingField = components["schemas"]["SettingField"];

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
 * - `nicknames:<communityId>`: the nicknames members chose in a community
 * - `overrides:<channelId|categoryId>`: one channel's or category's overrides
 * - `access:<communityId>`: what the caller may do across a community
 * - `channelAccess:<channelId>`: what the caller may do in one channel
 * - `pins:<channelId>`: a channel's pinned messages, once loaded
 * - `held:<channelId>`: the caller's messages there held for their attachments' previews
 * - `commands:<channelId>`: the commands of the bots that can see a channel, once loaded
 * - `annotations:<messageId>`: what plugins say about a message
 * - `userAnnotations:<userId>`: what plugins say about a person, once loaded
 * - `plugins`: the plugins the deployment runs
 * - `communityPlugins:<communityId>`: a community's use of each plugin, once loaded
 */
export type Topic = string;

/** The kinds of record fetched on demand whose absence the store remembers. */
export type MissingKind = "user" | "icon" | "poll" | "attachment";

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
  /**
   * Who a DM's call is ringing. A ring ends at its `until` with no event, so these include rings
   * that have run out; readers compare `until` with the clock.
   */
  readonly rings: readonly VoiceRing[];
}

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
