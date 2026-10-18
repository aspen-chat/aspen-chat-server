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
 * thread, a slice at a time, where workers cannot run.
 */
export const solveInWorker: PuzzleSolver = (challenge, difficulty) => {
  if (typeof Worker === "undefined") {
    return solvePuzzle(challenge, difficulty);
  }
  return new Promise((resolve) => {
    const worker = new Worker(new URL("./devicePuzzle.worker.ts", import.meta.url), {
      type: "module",
    });
    const onThisThread = () => {
      worker.terminate();
      resolve(solvePuzzle(challenge, difficulty));
    };
    worker.onmessage = (event: MessageEvent<number | null>) => {
      if (event.data === null) {
        onThisThread();
        return;
      }
      worker.terminate();
      resolve(event.data);
    };
    worker.onerror = onThisThread;
    worker.postMessage({ challenge, difficulty });
  });
};
