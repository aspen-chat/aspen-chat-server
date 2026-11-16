import { puzzleWork, solvePuzzleRange } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { MAX_PUZZLE_WORKERS, type PuzzleWorker, puzzleWorkerCount, solveAcross } from "./devices";
import type { PuzzleTask } from "./devicePuzzle.worker";

const challenge = new TextEncoder().encode("challenge");

/** How a stand-in worker answers the task it is given. */
type Behaviour = "solve" | "exhaust" | "error" | "messageError" | "silent" | "throw";

class FakeWorker implements PuzzleWorker {
  onmessage: ((event: MessageEvent<number | null>) => void) | null = null;
  onerror: ((event: ErrorEvent) => void) | null = null;
  onmessageerror: ((event: MessageEvent) => void) | null = null;
  task: PuzzleTask | null = null;
  terminated = false;
  readonly #behaviour: Behaviour;

  constructor(behaviour: Behaviour) {
    this.#behaviour = behaviour;
  }

  postMessage(task: PuzzleTask): void {
    this.task = task;
    queueMicrotask(() => {
      if (this.terminated) {
        return;
      }
      switch (this.#behaviour) {
        case "solve":
          this.answer(solvePuzzleRange(task.challenge, task.difficulty, task.from, task.to));
          return;
        case "exhaust":
          this.answer(null);
          return;
        case "error":
          this.onerror?.(new ErrorEvent("error"));
          return;
        case "messageError":
          this.onmessageerror?.(new MessageEvent("messageerror"));
          return;
        case "silent":
          return;
        case "throw":
          return;
      }
    });
  }

  answer(nonce: number | null): void {
    this.onmessage?.(new MessageEvent("message", { data: nonce }));
  }

  terminate(): void {
    this.terminated = true;
  }
}

/** Spawns a stand-in worker per behaviour, in order; `"throw"` makes spawning it throw. */
function spawner(behaviours: Behaviour[]) {
  const spawned: FakeWorker[] = [];
  let next = 0;
  const spawn = (): PuzzleWorker => {
    const behaviour = behaviours[next++] ?? "silent";
    if (behaviour === "throw") {
      throw new Error("workers refused");
    }
    const worker = new FakeWorker(behaviour);
    spawned.push(worker);
    return worker;
  };
  return { spawn, spawned };
}

describe("puzzleWorkerCount", () => {
  it("uses each core, up to the cap, and one when the count is unknown", () => {
    expect(puzzleWorkerCount(4)).toBe(4);
    expect(puzzleWorkerCount(64)).toBe(MAX_PUZZLE_WORKERS);
    expect(puzzleWorkerCount(0)).toBe(1);
    expect(puzzleWorkerCount(-3)).toBe(1);
    expect(puzzleWorkerCount(undefined)).toBe(1);
    expect(puzzleWorkerCount(Number.NaN)).toBe(1);
  });
});

describe("solveAcross", () => {
  it("gives each worker its own range and ends them all with the first answer", async () => {
    const { spawn, spawned } = spawner(["silent", "solve", "silent", "silent"]);
    const nonce = await solveAcross(spawn, 4)(challenge, 10);
    expect(spawned).toHaveLength(4);
    const [first, second] = spawned.map((worker) => worker.task);
    expect(nonce).toBeGreaterThanOrEqual(second?.from ?? 0);
    expect(nonce).toBeLessThan(second?.to ?? 0);
    expect(puzzleWork(challenge, nonce)).toBeGreaterThanOrEqual(10);
    expect(first?.to).toBe(second?.from);
    expect(spawned.every((worker) => worker.terminated)).toBe(true);
  });

  it("takes only the first of several answers", async () => {
    const { spawn, spawned } = spawner(["silent", "silent"]);
    const solving = solveAcross(spawn, 2)(challenge, 10);
    spawned[1]?.answer(7);
    spawned[0]?.answer(3);
    await expect(solving).resolves.toBe(7);
  });

  it("lets the others go on when a worker fails or finds nothing", async () => {
    const { spawn, spawned } = spawner(["error", "exhaust", "throw", "messageError", "solve"]);
    const nonce = await solveAcross(spawn, 5)(challenge, 10);
    expect(spawned).toHaveLength(4);
    const solver = spawned[3]?.task;
    expect(nonce).toBeGreaterThanOrEqual(solver?.from ?? 0);
    expect(puzzleWork(challenge, nonce)).toBeGreaterThanOrEqual(10);
    expect(spawned.every((worker) => worker.terminated)).toBe(true);
  });

  it("solves on the page's thread once every worker has ended without an answer", async () => {
    const { spawn, spawned } = spawner(["error", "exhaust", "messageError"]);
    const nonce = await solveAcross(spawn, 3)(challenge, 10);
    expect(nonce).toBe(solvePuzzleRange(challenge, 10, 0, Number.MAX_SAFE_INTEGER));
    expect(spawned.every((worker) => worker.terminated)).toBe(true);
  });

  it("solves on the page's thread when no worker can be started", async () => {
    const { spawn } = spawner(["throw", "throw"]);
    const nonce = await solveAcross(spawn, 2)(challenge, 10);
    expect(nonce).toBe(solvePuzzleRange(challenge, 10, 0, Number.MAX_SAFE_INTEGER));
  });

  it("ends every worker and rejects when aborted", async () => {
    const { spawn, spawned } = spawner(["silent", "silent", "silent"]);
    const controller = new AbortController();
    const solving = solveAcross(spawn, 3)(challenge, 10, controller.signal);
    const reason = new Error("gave up");
    controller.abort(reason);
    await expect(solving).rejects.toBe(reason);
    expect(spawned.every((worker) => worker.terminated)).toBe(true);
    spawned[0]?.answer(1);
  });

  it("starts no worker when already aborted", async () => {
    const { spawn, spawned } = spawner(["solve"]);
    const controller = new AbortController();
    controller.abort();
    await expect(solveAcross(spawn, 1)(challenge, 10, controller.signal)).rejects.toBeDefined();
    expect(spawned).toHaveLength(0);
  });

  it("is not aborted by its signal after it settles", async () => {
    const { spawn } = spawner(["solve"]);
    const controller = new AbortController();
    const nonce = await solveAcross(spawn, 1)(challenge, 10, controller.signal);
    controller.abort();
    expect(puzzleWork(challenge, nonce)).toBeGreaterThanOrEqual(10);
  });
});
