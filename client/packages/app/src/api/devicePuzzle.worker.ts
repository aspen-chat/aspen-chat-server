/**
 * Solves a device credential's puzzle off the page's thread (`devices.ts`): it takes about a
 * million hashes, a second or two on a phone, which would otherwise freeze the page that long.
 */
import { solvePuzzleRange } from "@aspen/protocol/device";

interface Puzzle {
  challenge: Uint8Array;
  difficulty: number;
}

const scope = globalThis as unknown as {
  onmessage: ((event: MessageEvent<Puzzle>) => void) | null;
  postMessage: (nonce: number | null) => void;
};

scope.onmessage = (event) => {
  const { challenge, difficulty } = event.data;
  scope.postMessage(solvePuzzleRange(challenge, difficulty, 0, Number.MAX_SAFE_INTEGER));
};
