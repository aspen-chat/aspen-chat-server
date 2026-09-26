import type { VoiceParticipantState } from "@aspen/protocol";

/** The most participants a channel's list shows under it. */
export const VOICE_LIST_LIMIT = 15;

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
    .sort(
      (a, b) =>
        (b.lastSpokeAt ?? -Infinity) - (a.lastSpokeAt ?? -Infinity) ||
        a.joinedAt.localeCompare(b.joinedAt),
    )
    .slice(0, limit);
}
