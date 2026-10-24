import {
  PasskeyCancelledError,
  PasskeyUnavailableError,
  type AuthMethods,
  type NativePasskeys,
} from "@aspen/protocol";
import { Capacitor, registerPlugin } from "@capacitor/core";

/**
 * The Android app's own side of passkeys (`AspenPasskeysPlugin`): Credential Manager, which runs
 * a deployment's ceremonies in the app itself once the deployment names the app in its
 * `assetlinks.json`. A failure rejects with one of `PasskeyFailure`'s codes.
 */
interface AspenPasskeysPlugin {
  identity(): Promise<{ packageName: string; available: boolean; fingerprints: string[] }>;
  create(options: { publicKey: unknown }): Promise<{ credential: Record<string, unknown> }>;
  get(options: { publicKey: unknown }): Promise<{ credential: Record<string, unknown> }>;
}

const AspenPasskeys = registerPlugin<AspenPasskeysPlugin>("AspenPasskeys");

/** What a rejection from the plugin means here. */
function failure(e: unknown): Error {
  const code = (e as { code?: unknown } | null)?.code;
  if (code === "cancelled") {
    return new PasskeyCancelledError();
  }
  if (code === "unavailable") {
    return new PasskeyUnavailableError(e instanceof Error ? e.message : String(e));
  }
  return e instanceof Error ? e : new Error(String(e));
}

const native: NativePasskeys = {
  async create(publicKey) {
    try {
      return (await AspenPasskeys.create({ publicKey })).credential;
    } catch (e) {
      throw failure(e);
    }
  },
  async get(publicKey) {
    try {
      return (await AspenPasskeys.get({ publicKey })).credential;
    } catch (e) {
      throw failure(e);
    }
  },
};

/**
 * Credential Manager, where this is the Android app, it can offer passkeys, and the deployment
 * names this app with a certificate it is signed with (`passkeys.androidApps`, which a
 * deployment that predates it leaves out); `null` otherwise, and the app hands ceremonies to the
 * system browser.
 */
export async function nativePasskeys(methods: AuthMethods): Promise<NativePasskeys | null> {
  if (Capacitor.getPlatform() !== "android" || !Capacitor.isPluginAvailable("AspenPasskeys")) {
    return null;
  }
  const named = methods.passkeys?.androidApps ?? [];
  if (named.length === 0) {
    return null;
  }
  const identity = await AspenPasskeys.identity().catch(() => null);
  if (!identity?.available) {
    return null;
  }
  const ours = named.some(
    (app) =>
      app.packageName === identity.packageName &&
      identity.fingerprints.includes(app.sha256CertFingerprint.toUpperCase()),
  );
  return ours ? native : null;
}
