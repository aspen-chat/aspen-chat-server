import { PasskeyCancelledError, PasskeyUnavailableError, type AuthMethods } from "@aspen/protocol";
import { beforeEach, describe, expect, it, vi } from "vitest";

const FINGERPRINT = Array.from({ length: 32 }, () => "AB").join(":");

let platform = "android";
let identity = {
  packageName: "com.aspen_chat.client",
  available: true,
  fingerprints: [FINGERPRINT],
};
let rejection: Error | null = null;
const asked: string[] = [];

vi.mock("@capacitor/core", () => ({
  Capacitor: {
    getPlatform: () => platform,
    isPluginAvailable: (name: string) => name === "AspenPasskeys",
  },
  registerPlugin: () => ({
    identity: () => Promise.resolve(identity),
    get: (options: { host: string }) => {
      asked.push(options.host);
      return rejection === null
        ? Promise.resolve({ credential: { id: "c" } })
        : Promise.reject(rejection);
    },
    create: () => Promise.reject(rejection ?? new Error("not asked")),
  }),
}));

const { nativePasskeys } = await import("./nativePasskeys");
const { resolvePasskeyTransport } = await import("./passkeyTransport");

function methods(androidApps?: { packageName: string; sha256CertFingerprint: string }[]) {
  return {
    twoFactorRequired: false,
    registrationInviteRequired: false,
    protocol: { version: 1, minimum: 1 },
    software: { name: "aspen", version: "0.1.0" },
    passkeys: { rpId: "chat.example.org", ...(androidApps === undefined ? {} : { androidApps }) },
  } as AuthMethods;
}

const named = [
  { packageName: "com.aspen_chat.client", sha256CertFingerprint: FINGERPRINT.toLowerCase() },
];

describe("nativePasskeys", () => {
  beforeEach(() => {
    platform = "android";
    identity = {
      packageName: "com.aspen_chat.client",
      available: true,
      fingerprints: [FINGERPRINT],
    };
    rejection = null;
  });

  it("are used where the deployment names this app and the certificate it is signed with", async () => {
    expect(await nativePasskeys(methods(named))).not.toBeNull();
    const transport = await resolvePasskeyTransport("mobile", methods(named));
    expect(transport?.kind).toBe("native");
    expect(transport?.kind === "native" ? transport.fallback : null).not.toBeNull();
  });

  it("are not used where the deployment names no app, another, or predates naming them", async () => {
    expect(await nativePasskeys(methods())).toBeNull();
    expect(await nativePasskeys(methods([]))).toBeNull();
    expect(
      await nativePasskeys(
        methods([{ packageName: "org.example.fork", sha256CertFingerprint: FINGERPRINT }]),
      ),
    ).toBeNull();
    identity = { ...identity, fingerprints: ["00:11"] };
    expect(await nativePasskeys(methods(named))).toBeNull();
    expect((await resolvePasskeyTransport("mobile", methods(named)))?.kind).toBe("handoff");
  });

  it("are not used off Android or where Credential Manager offers no passkeys", async () => {
    identity = { ...identity, available: false };
    expect(await nativePasskeys(methods(named))).toBeNull();
    identity = { ...identity, available: true };
    platform = "ios";
    expect(await nativePasskeys(methods(named))).toBeNull();
  });

  it("read the plugin's failures as the ceremony needs them", async () => {
    const native = await nativePasskeys(methods(named));
    rejection = Object.assign(new Error("dismissed"), { code: "cancelled" });
    await expect(native?.get({}, "chat.example.org")).rejects.toBeInstanceOf(PasskeyCancelledError);
    rejection = Object.assign(new Error("no provider"), { code: "unavailable" });
    await expect(native?.create({}, "chat.example.org")).rejects.toBeInstanceOf(
      PasskeyUnavailableError,
    );
    rejection = Object.assign(new Error("broken"), { code: "failed" });
    await expect(native?.get({}, "chat.example.org")).rejects.toThrow("broken");
    rejection = null;
    await expect(native?.get({}, "chat.example.org")).resolves.toEqual({ id: "c" });
    // The plugin is told whose ceremony it runs, and checks the relying party against it.
    expect(asked.at(-1)).toBe("chat.example.org");
  });
});
