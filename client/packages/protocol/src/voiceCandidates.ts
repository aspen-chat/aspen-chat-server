/** Ranking a join offer's voice server candidates, and the signalling URL of each. */

import type { components } from "./generated/openapi";

type VoiceServerCandidate = components["schemas"]["VoiceServerCandidate"];

/** A candidate with the round trip to it, `Infinity` when it did not answer. */
export interface RankedCandidate {
  candidate: VoiceServerCandidate;
  latencyMs: number;
}

/** Nearest first; candidates that did not answer come last, in their offered order. */
export function rankCandidates(ranked: readonly RankedCandidate[]): RankedCandidate[] {
  return [...ranked].sort((a, b) => a.latencyMs - b.latencyMs);
}

/** The signalling URL of a candidate: its `/ws` over the WebSocket scheme matching its own. */
export function signallingUrl(candidate: VoiceServerCandidate): string {
  const url = new URL(candidate.url);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  url.pathname = `${url.pathname.replace(/\/$/, "")}/ws`;
  return url.toString();
}
