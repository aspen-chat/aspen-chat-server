import type { AspenClient, User } from "@aspen/protocol";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

let passkeys: { credentialId: string }[] = [{ credentialId: "cred" }];
let reads = 0;

vi.mock("@/features/security/api", () => ({
  fetchSecurity: () => {
    reads += 1;
    return Promise.resolve({ passkeys, passkeyUserHandle: "handle" });
  },
}));

const { signalProfileToPasskeys } = await import("./passkeySignals");

const client = {} as AspenClient;
const user = { id: "u", name: "kate", displayName: "Kate K" } as User;
const inPage = { kind: "inPage", rpId: "chat.example.org" } as const;

describe("signalProfileToPasskeys", () => {
  let sent: unknown[] = [];

  beforeEach(() => {
    sent = [];
    reads = 0;
    passkeys = [{ credentialId: "cred" }];
    vi.stubGlobal("PublicKeyCredential", {
      signalCurrentUserDetails: (options: unknown) => {
        sent.push(options);
        return Promise.resolve();
      },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("names the account's username and name to the browser", async () => {
    await signalProfileToPasskeys(client, inPage, user);
    expect(sent).toEqual([
      { rpId: "chat.example.org", userId: "handle", name: "kate", displayName: "Kate K" },
    ]);
    await signalProfileToPasskeys(client, inPage, { ...user, displayName: null });
    expect(sent.at(-1)).toMatchObject({ displayName: "kate" });
  });

  it("sends nothing for an account without passkeys", async () => {
    passkeys = [];
    await signalProfileToPasskeys(client, inPage, user);
    expect(sent).toEqual([]);
  });

  it("reads nothing where the page may not send it or the browser lacks it", async () => {
    await signalProfileToPasskeys(client, null, user);
    await signalProfileToPasskeys(client, { kind: "handoff", handoff: { prepare: vi.fn() } }, user);
    vi.stubGlobal("PublicKeyCredential", {});
    await signalProfileToPasskeys(client, inPage, user);
    expect(reads).toBe(0);
    expect(sent).toEqual([]);
  });
});
