import type { SourceResults } from "@/features/search/merge";

/** A channel or DM with something unread on one deployment (`key`), and its newest unread message. */
export interface UnreadPlace {
  readonly key: string;
  readonly channelId: string;
  readonly newest: string;
}

/** An entry of the Unread tab: a channel or DM, or a thread the reader follows. */
export interface UnreadEntry extends UnreadPlace {
  readonly thread: boolean;
}

/**
 * The Unread tab's entries, newest first: every unread channel and DM (`channels`, which each
 * deployment's store holds whole), and each followed thread with unread replies in `threads`
 * (each deployment's unread replies so far, newest first), a thread standing once, where its
 * newest reply does. A deployment with more replies to give may still have threads newer than
 * its oldest reply read, so nothing older than the oldest reply of any such deployment is shown
 * until it has given its next page. Ids are UUIDv7s, which sort by time as text on every
 * deployment.
 */
export function unreadEntries(
  channels: readonly UnreadPlace[],
  threads: readonly SourceResults<{ id: string; timestamp: string; channelId: string }>[],
): UnreadEntry[] {
  const cutoff = threads
    .filter((feed) => !feed.exhausted)
    .map((feed) => feed.messages.at(-1)?.id ?? "")
    .reduce((newest, id) => (id > newest ? id : newest), "");
  const folded = new Map<string, UnreadEntry>();
  for (const feed of threads) {
    for (const reply of feed.messages) {
      const place = `${feed.key}/${reply.channelId}`;
      const held = folded.get(place);
      if (held === undefined || reply.id > held.newest) {
        folded.set(place, {
          key: feed.key,
          channelId: reply.channelId,
          newest: reply.id,
          thread: true,
        });
      }
    }
  }
  return [...channels.map((channel) => ({ ...channel, thread: false })), ...folded.values()]
    .filter((entry) => entry.newest >= cutoff)
    .sort((a, b) => (a.newest < b.newest ? 1 : a.newest > b.newest ? -1 : 0));
}
