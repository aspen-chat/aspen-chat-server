import { afterEach, describe, expect, it, vi } from "vitest";
import { passkeyTransport } from "./passkeyTransport";

const server = {
  twoFactorRequired: false,
  registrationInviteRequired: false,
  protocol: { version: 1, minimum: 1 },
  software: { name: "aspen", version: "0.1.0" },
};
const withPasskeys = { ...server, passkeys: { rpId: "localhost" } };
const withoutPasskeys = { ...server, passkeys: null };

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("passkeyTransport", () => {
  it("offers nothing when the server has no passkeys", () => {
    expect(passkeyTransport("web", withoutPasskeys)).toBeNull();
    expect(passkeyTransport("mobile", withoutPasskeys)).toBeNull();
  });

  it("runs in the page when the web client is under the relying party's domain", () => {
    // jsdom serves the page from localhost.
    vi.stubGlobal("PublicKeyCredential", {});
    expect(passkeyTransport("web", withPasskeys)).toEqual({ kind: "inPage" });
    expect(
      passkeyTransport("web", { ...server, passkeys: { rpId: "chat.example.org" } }),
    ).toBeNull();
  });

  it("offers nothing in a browser without WebAuthn", () => {
    expect(passkeyTransport("web", withPasskeys)).toBeNull();
  });

  it("hands ceremonies to the system browser from the shells", () => {
    expect(passkeyTransport("mobile", withPasskeys)?.kind).toBe("handoff");
    // Without the preload bridge the desktop shell cannot listen for the return.
    expect(passkeyTransport("desktop", withPasskeys)).toBeNull();
  });
});
