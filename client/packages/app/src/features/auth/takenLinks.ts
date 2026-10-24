import { ApiProblemError, type AspenClient, type DeviceLinkScan } from "@aspen/protocol";

/** A link this device took, by scanning its QR code or typing its user code (`typed`). */
export interface Taken {
  scan: DeviceLinkScan;
  verifier: string | null;
  typed?: boolean;
}

/**
 * Each code this page scanned, or typed (`TypeDeviceLinkCode`), by id: taking uses a code up, so
 * a screen that mounts twice (React's development double render, or the route rendering again
 * once signed in) reuses the one scan rather than meeting `deviceLinkUsed` from its own first.
 */
const scans = new Map<string, Promise<Taken>>();

/** Keeps a link taken by typing its code for `DeviceLinkScreen` to go on with. */
export function rememberTyped(id: string, scan: DeviceLinkScan, verifier: string | null) {
  scans.set(id, Promise.resolve({ scan, verifier, typed: true }));
}

/** Scans the code `id` once, or answers what this page took it with before. */
export function scanOnce(client: AspenClient, id: string, deviceName: string): Promise<Taken> {
  let scan = scans.get(id);
  if (scan === undefined) {
    scan = client.scanDeviceLink(id, deviceName);
    scans.set(id, scan);
    // A scan refused before it used the code (a sign-in to verify again first) may be tried
    // again, so it is not kept.
    scan.catch((e: unknown) => {
      if (e instanceof ApiProblemError && e.code === "reauthenticationRequired") {
        scans.delete(id);
      }
    });
  }
  return scan;
}
