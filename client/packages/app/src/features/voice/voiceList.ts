import type { VoiceParticipantState } from "@aspen/protocol";

/** The most participants a channel's list shows under it. */
export const VOICE_LIST_LIMIT = 15;

/** The nanoseconds past the second of an RFC 3339 time, of which a `Date` keeps only the first three digits. */
function nanoseconds(time: string): number {
  const digits = /\.(\d+)/.exec(time)?.[1] ?? "";
  return Number(digits.padEnd(9, "0").slice(0, 9));
}

/**
 * Who joined first, by their `joinedAt` as times and then by user id, so two who joined at one
 * moment keep one order. The times are not compared as text: the server writes as many
 * fractional digits as each has, and a shorter one sorts after a longer one it begins.
 */
function joinOrder(a: VoiceParticipantState, b: VoiceParticipantState): number {
  return (
    Date.parse(a.joinedAt) - Date.parse(b.joinedAt) ||
    nanoseconds(a.joinedAt) - nanoseconds(b.joinedAt) ||
    (a.user < b.user ? -1 : a.user > b.user ? 1 : 0)
  );
}

/**
 * Who to show under a voice channel. Everyone, in the order they joined, while the call has no
 * more than `limit` people; past that, the `limit` who spoke most recently, with those who
 * never spoke coming last by join time, so the people who are actually talking stay visible.
 */
export function visibleParticipants(
  participants: readonly VoiceParticipantState[],
  limit = VOICE_LIST_LIMIT,
): VoiceParticipantState[] {
  if (participants.length <= limit) {
    return [...participants];
  }
  return [...participants]
    .sort((a, b) => (b.lastSpokeAt ?? -Infinity) - (a.lastSpokeAt ?? -Infinity) || joinOrder(a, b))
    .slice(0, limit);
}
