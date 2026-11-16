import {
  type ClockOffsetStore,
  type DeviceCredentialStore,
  type PuzzleSolver,
  puzzleRanges,
  solvePuzzle,
  WebStorageClockOffsetStore,
  WebStorageDeviceCredentialStore,
} from "@aspen/protocol";
import type { PuzzleTask } from "./devicePuzzle.worker";

/**
 * Where the device credential for the deployment at `baseUrl` is kept: one key per deployment,
 * so changing the server never presents one server's credential to another.
 */
export function deviceStore(baseUrl: string): DeviceCredentialStore {
  return new WebStorageDeviceCredentialStore(
    window.localStorage,
    `aspen.device.${new URL(baseUrl).host}`,
  );
}

/**
 * Where the clock offset of the deployment at `baseUrl` is kept between runs: one key per
 * deployment, so no deployment's clock is judged by another's offset.
 */
export function clockStore(baseUrl: string): ClockOffsetStore {
  return new WebStorageClockOffsetStore(
    window.localStorage,
    `aspen.clock.${new URL(baseUrl).host}`,
  );
}

/**
 * The most workers one puzzle is solved across. The expected time falls with each, but at eight
 * the hashing alone takes under a tenth of a second on a desktop, while every worker more loads
 * the solver's module again and holds memory of its own for as long as the puzzle takes.
 */
export const MAX_PUZZLE_WORKERS = 8;

/**
 * How many workers to solve a puzzle across: one per logical core (`hardwareConcurrency`, which
 * a browser may leave out), up to `MAX_PUZZLE_WORKERS`.
 */
export function puzzleWorkerCount(cores: number | undefined): number {
  if (cores === undefined || !Number.isFinite(cores)) {
    return 1;
  }
  return Math.min(MAX_PUZZLE_WORKERS, Math.max(1, Math.floor(cores)));
}

/** What `solveAcross` uses of a `Worker` running `devicePuzzle.worker.ts`. */
export interface PuzzleWorker {
  onmessage: ((event: MessageEvent<number | null>) => void) | null;
  onerror: ((event: ErrorEvent) => void) | null;
  onmessageerror: ((event: MessageEvent) => void) | null;
  postMessage(task: PuzzleTask): void;
  terminate(): void;
}

/**
 * Solves a puzzle across `count` workers from `spawn`, each searching its own range of the
 * nonces (`puzzleRanges`), so the expected time and its spread both shrink with the count. The
 * first nonce any worker finds is the answer, and every worker is then ended; the server takes
 * any nonce that solves the puzzle, not only the smallest.
 *
 * A worker that cannot be started, errors, or finds nothing in its range ends alone and the rest
 * go on. Only once every worker has ended without an answer is the puzzle solved on the page's
 * thread instead, a slice at a time (`solvePuzzle`). Aborting `signal` ends every worker and
 * rejects.
 */
export function solveAcross(spawn: () => PuzzleWorker, count: number): PuzzleSolver {
  return (challenge, difficulty, signal) =>
    new Promise((resolve, reject) => {
      const workers = new Set<PuzzleWorker>();
      let starting = true;
      let settled = false;
      const settle = (settles: () => void) => {
        if (settled) {
          return;
        }
        settled = true;
        signal?.removeEventListener("abort", stop);
        for (const worker of workers) {
          worker.terminate();
        }
        workers.clear();
        settles();
      };
      const stop = () => {
        settle(() => {
          reject(signal?.reason instanceof Error ? signal.reason : new Error("puzzle abandoned"));
        });
      };
      const onThisThread = () => {
        settle(() => {
          resolve(solvePuzzle(challenge, difficulty, signal));
        });
      };
      const end = (worker: PuzzleWorker) => {
        worker.terminate();
        workers.delete(worker);
        if (!starting && workers.size === 0) {
          onThisThread();
        }
      };
      if (signal?.aborted === true) {
        stop();
        return;
      }
      signal?.addEventListener("abort", stop, { once: true });
      for (const { from, to } of puzzleRanges(count)) {
        let worker: PuzzleWorker;
        try {
          worker = spawn();
        } catch {
          continue;
        }
        workers.add(worker);
        worker.onmessage = (event) => {
          const nonce = event.data;
          if (nonce === null) {
            end(worker);
            return;
          }
          settle(() => {
            resolve(nonce);
          });
        };
        worker.onerror = () => {
          end(worker);
        };
        worker.onmessageerror = () => {
          end(worker);
        };
        try {
          worker.postMessage({ challenge, difficulty, from, to });
        } catch {
          end(worker);
        }
      }
      starting = false;
      if (workers.size === 0) {
        onThisThread();
      }
    });
}

/**
 * Solves a device credential's puzzle across workers (`devicePuzzle.worker.ts`), one per core up
 * to `MAX_PUZZLE_WORKERS`, started for each puzzle and ended with it, or on the page's thread, a
 * slice at a time, where workers cannot run.
 */
export const solveInWorkers: PuzzleSolver = (challenge, difficulty, signal) => {
  if (typeof Worker === "undefined") {
    return solvePuzzle(challenge, difficulty, signal);
  }
  const spawn = () =>
    new Worker(new URL("./devicePuzzle.worker.ts", import.meta.url), { type: "module" });
  return solveAcross(spawn, puzzleWorkerCount(navigator.hardwareConcurrency))(
    challenge,
    difficulty,
    signal,
  );
};
