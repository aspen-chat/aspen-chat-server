import { describe, expect, it } from "vitest";
import {
  canRunInPage,
  creationOptions,
  fromBase64url,
  parseHandoffReturn,
  requestOptions,
  toBase64url,
} from "../src";

describe("passkeys", () => {
  it("round-trips base64url without padding", () => {
    for (const length of [0, 1, 2, 3, 31, 32]) {
      const bytes = Uint8Array.from({ length }, (_, i) => (i * 37 + 250) % 256);
      const text = toBase64url(bytes);
      expect(text).not.toMatch(/[+/=]/);
      expect(Array.from(fromBase64url(text))).toEqual(Array.from(bytes));
    }
  });

  it("decodes the binary fields of registration options", () => {
    const options = creationOptions({
      publicKey: {
        challenge: "AAEC",
        rp: { id: "localhost", name: "Aspen" },
        user: { id: "AwQF", name: "kate", displayName: "Kate" },
        pubKeyCredParams: [{ type: "public-key", alg: -7 }],
        excludeCredentials: [{ id: "BgcI", type: "public-key" }],
      },
    });
    expect(Array.from(options.challenge as Uint8Array)).toEqual([0, 1, 2]);
    expect(Array.from(options.user.id as Uint8Array)).toEqual([3, 4, 5]);
    expect(Array.from(options.excludeCredentials?.[0]?.id as Uint8Array)).toEqual([6, 7, 8]);
    expect(options.rp.id).toBe("localhost");
  });

  it("decodes the binary fields of authentication options", () => {
    const options = requestOptions({
      publicKey: { challenge: "AAEC", rpId: "localhost", allowCredentials: [] },
    });
    expect(Array.from(options.challenge as Uint8Array)).toEqual([0, 1, 2]);
    expect(options.allowCredentials).toEqual([]);
    expect(() => requestOptions({})).toThrow();
  });

  it("runs in page only under the relying party's domain", () => {
    expect(canRunInPage("example.org", "example.org", true)).toBe(true);
    expect(canRunInPage("example.org", "chat.example.org", true)).toBe(true);
    expect(canRunInPage("example.org", "badexample.org", true)).toBe(false);
    expect(canRunInPage("example.org", "example.org", false)).toBe(false);
  });

  it("reads the browser's return", () => {
    expect(parseHandoffReturn("http://127.0.0.1:5000/passkey?ceremony=abc&outcome=done")).toEqual({
      ceremony: "abc",
      outcome: "done",
    });
    expect(parseHandoffReturn("aspen://auth/passkey?ceremony=abc&outcome=cancelled")).toEqual({
      ceremony: "abc",
      outcome: "cancelled",
    });
    expect(parseHandoffReturn("http://127.0.0.1:5000/favicon.ico")).toBeNull();
    expect(parseHandoffReturn("not a url")).toBeNull();
  });
});
