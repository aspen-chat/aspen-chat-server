import type { VoiceParticipantState } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { VOICE_LIST_LIMIT, visibleParticipants } from "./voiceList";

function person(n: number, lastSpokeAt: number | null): VoiceParticipantState {
  return {
    user: `u${String(n)}`,
    session: "s",
    channel: "c",
    joinedAt: `2026-09-26T00:00:${String(n).padStart(2, "0")}Z`,
    muted: false,
    deafened: false,
    sharingScreen: false,
    speaking: false,
    lastSpokeAt,
  };
}

describe("visibleParticipants", () => {
  it("shows everyone in join order up to the limit", () => {
    const few = [person(3, 9), person(1, null), person(2, 4)];
    expect(visibleParticipants(few).map((p) => p.user)).toEqual(["u3", "u1", "u2"]);
  });

  it("past the limit keeps the most recent speakers, never-spoke last by join time", () => {
    const many = Array.from({ length: VOICE_LIST_LIMIT + 5 }, (_, i) =>
      person(i, i % 4 === 0 ? null : 100 - i),
    );
    const shown = visibleParticipants(many);
    expect(shown).toHaveLength(VOICE_LIST_LIMIT);
    const spoke = many
      .filter((p) => p.lastSpokeAt !== null)
      .sort((a, b) => (b.lastSpokeAt ?? 0) - (a.lastSpokeAt ?? 0));
    expect(shown.map((p) => p.user)).toEqual(spoke.slice(0, VOICE_LIST_LIMIT).map((p) => p.user));
    const silentOnly = Array.from({ length: VOICE_LIST_LIMIT + 2 }, (_, i) => person(i, null));
    expect(visibleParticipants(silentOnly).map((p) => p.user)).toEqual(
      silentOnly.slice(0, VOICE_LIST_LIMIT).map((p) => p.user),
    );
  });
});
