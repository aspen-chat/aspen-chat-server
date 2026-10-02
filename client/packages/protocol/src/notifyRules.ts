/**
 * What the caller is told of, and what counts as unread, decided from the records
 * `RecordStore` holds. The store reads its own state and memoizes; these decide.
 */

import { isDm } from "./channels";
import type { Channel, Message } from "./generated/events";
import type { ChannelMute, Mentions, NotificationLevel, ReadState } from "./storeTypes";

/** Stands for the DMs among `RecordStore.unreadPlaces`, beside community ids. */
export const UNREAD_DMS = "dms";

/** Whether a read state holds a message by someone else not yet read. */
export function isUnread(state: ReadState | undefined): boolean {
  return state?.lastMessage != null && state.lastMessage > state.lastRead;
}

/**
 * The unread tags of the caller across a community's channels, or across their DMs with
 * `UNREAD_DMS`.
 */
export function placeMentions(
  place: string,
  readStates: ReadonlyMap<string, ReadState>,
  channels: ReadonlyMap<string, Channel>,
): number {
  let total = 0;
  for (const state of readStates.values()) {
    const channel = channels.get(state.channel);
    const home = channel === undefined ? undefined : isDm(channel) ? UNREAD_DMS : channel.community;
    if (home === place) {
      total += state.mentions;
    }
  }
  return total;
}

/**
 * The level a channel notifies at, given the channel whose setting governs it (a thread's
 * parent), its own setting, and its community's: the channel's, else the community's, else
 * every message of a DM and tags elsewhere.
 */
export function levelOf(
  place: Channel | undefined,
  own: NotificationLevel | null,
  community: NotificationLevel | undefined,
): { level: NotificationLevel; own: NotificationLevel | null; inherited: NotificationLevel } {
  const fallback: NotificationLevel =
    place?.ty === "dm" || place?.ty === "groupDm" ? "all" : "tags";
  const inherited = community ?? fallback;
  return { level: own ?? inherited, own, inherited };
}

/** Whether a message's kind is one that may notify at all. */
export function kindNotifies(message: Message): boolean {
  // An echo and a poll's result say nothing of their own, a call's record follows the ring
  // that already told of the call, and a command is for its bot, whose answer is what tells.
  if (
    message.kind === "threadEcho" ||
    message.kind === "pollClosed" ||
    message.kind === "call" ||
    message.kind === "missedCall" ||
    message.kind === "command"
  ) {
    return false;
  }
  return true;
}

/**
 * Whether a message's tags name `me`: by name, through one of the roles `held` in its
 * community, or as everyone.
 */
export function tagsMe(tags: Mentions, me: string, held: readonly string[] | undefined): boolean {
  if (tags.everyone || tags.users.includes(me)) {
    return true;
  }
  return held !== undefined && tags.roles.some((role) => held.includes(role));
}

/**
 * The communities with an unread channel, and `UNREAD_DMS` when a DM is unread. A muted
 * channel counts for neither.
 */
export function unreadPlaces(
  readStates: ReadonlyMap<string, ReadState>,
  channels: ReadonlyMap<string, Channel>,
  mutes: ReadonlyMap<string, ChannelMute>,
): Set<string> {
  const places = new Set<string>();
  for (const state of readStates.values()) {
    const channel = channels.get(state.channel);
    if (
      channel !== undefined &&
      !mutes.has(state.channel) &&
      isUnread(readStates.get(state.channel))
    ) {
      places.add(channel.community ?? UNREAD_DMS);
    }
  }
  return places;
}
