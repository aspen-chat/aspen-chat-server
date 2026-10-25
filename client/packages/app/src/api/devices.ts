import {
  type DeviceCredentialStore,
  type PuzzleSolver,
  solvePuzzle,
  WebStorageDeviceCredentialStore,
} from "@aspen/protocol";

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
 * Solves a device credential's puzzle in a worker (`devicePuzzle.worker.ts`), or on the page's
 * thread, a slice at a time, where workers cannot run. Aborting `signal` ends the worker.
 */
export const solveInWorker: PuzzleSolver = (challenge, difficulty, signal) => {
  if (typeof Worker === "undefined") {
    return solvePuzzle(challenge, difficulty, signal);
  }
  return new Promise((resolve, reject) => {
    const worker = new Worker(new URL("./devicePuzzle.worker.ts", import.meta.url), {
      type: "module",
    });
    const stop = () => {
      worker.terminate();
      reject(signal?.reason instanceof Error ? signal.reason : new Error("puzzle abandoned"));
    };
    if (signal?.aborted === true) {
      stop();
      return;
    }
    signal?.addEventListener("abort", stop, { once: true });
    const settle = (settled: () => void) => {
      signal?.removeEventListener("abort", stop);
      worker.terminate();
      settled();
    };
    const onThisThread = () => {
      settle(() => {
        resolve(solvePuzzle(challenge, difficulty, signal));
      });
    };
    worker.onmessage = (event: MessageEvent<number | null>) => {
      if (event.data === null) {
        onThisThread();
        return;
      }
      const nonce = event.data;
      settle(() => {
        resolve(nonce);
      });
    };
    worker.onerror = onThisThread;
    worker.postMessage({ challenge, difficulty });
  });
};
