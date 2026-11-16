/**
 * Searches one range of a device credential's puzzle off the page's thread (`devices.ts`, which
 * runs several of these side by side): a puzzle takes about a million hashes, seconds on a phone,
 * which would otherwise freeze the page that long. Answers the first nonce in its range that
 * solves it, or `null` once the range holds none.
 */
import { type NonceRange, solvePuzzleRange } from "@aspen/protocol/device";

export interface PuzzleTask extends NonceRange {
  challenge: Uint8Array;
  difficulty: number;
}

const scope = globalThis as unknown as {
  onmessage: ((event: MessageEvent<PuzzleTask>) => void) | null;
  postMessage: (nonce: number | null) => void;
};

scope.onmessage = (event) => {
  const { challenge, difficulty, from, to } = event.data;
  scope.postMessage(solvePuzzleRange(challenge, difficulty, from, to));
};
