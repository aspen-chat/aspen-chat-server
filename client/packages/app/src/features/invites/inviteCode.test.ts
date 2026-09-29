import { describe, expect, it } from "vitest";
import { parseInvite } from "./inviteCode";

describe("parseInvite", () => {
  it("accepts bare codes and links from either history mode", () => {
    const here = (code: string) => ({ code, domain: null });
    expect(parseInvite("  abc123 ")).toEqual(here("abc123"));
    expect(parseInvite("https://chat.example.org/invite/abc123")).toEqual(here("abc123"));
    expect(parseInvite("https://chat.example.org/invite/abc123/")).toEqual(here("abc123"));
    expect(parseInvite("file:///opt/aspen/index.html#/invite/abc123")).toEqual(here("abc123"));
    expect(parseInvite("http://localhost:5173/invite/abc123?utm=1")).toEqual(here("abc123"));
  });

  it("reads the deployment a link names", () => {
    const at = (domain: string) => ({ code: "abc123", domain });
    expect(parseInvite("https://a.example/invite/abc123?at=b.example")).toEqual(at("b.example"));
    expect(parseInvite("https://a.example/invite/abc123?utm=1&at=B.Example:8443")).toEqual(
      at("b.example:8443"),
    );
    expect(parseInvite("file:///aspen/index.html#/invite/abc123?at=b.example%3A8443")).toEqual(
      at("b.example:8443"),
    );
    expect(parseInvite("https://a.example/at/b.example%3A8443/invite/abc123")).toEqual(
      at("b.example:8443"),
    );
  });

  it("rejects anything that is not a code", () => {
    expect(parseInvite("")).toBeNull();
    expect(parseInvite("not a code")).toBeNull();
    expect(parseInvite("https://chat.example.org/communities/x")).toBeNull();
    expect(parseInvite("abcdefghijklmnopq")).toBeNull();
  });
});
