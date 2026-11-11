import { describe, expect, it } from "vitest";
import { namedInvite, parseInvite } from "./inviteCode";

/** The addresses of the deployment in use. */
const HOME = ["chat.example.org", "localhost:5173"];
const parse = (input: string) => parseInvite(input, HOME);

describe("parseInvite", () => {
  it("accepts bare codes and links from either history mode", () => {
    const here = (code: string) => ({ code, domain: null });
    expect(parse("  abc123 ")).toEqual(here("abc123"));
    expect(parse("https://chat.example.org/invite/abc123")).toEqual(here("abc123"));
    expect(parse("https://chat.example.org/invite/abc123/")).toEqual(here("abc123"));
    expect(parse("https://Chat.Example.org/invite/abc123")).toEqual(here("abc123"));
    expect(parse("file:///opt/aspen/index.html#/invite/abc123")).toEqual(here("abc123"));
    expect(parse("http://localhost:5173/invite/abc123?utm=1")).toEqual(here("abc123"));
    expect(parse("http://localhost:5173/#/invite/abc123")).toEqual(here("abc123"));
    expect(parse("aspen://app/invite/abc123")).toEqual(here("abc123"));
  });

  it("reads the deployment a link names", () => {
    const at = (domain: string) => ({ code: "abc123", domain });
    expect(parse("https://a.example/invite/abc123?at=b.example")).toEqual(at("b.example"));
    expect(parse("https://a.example/invite/abc123?utm=1&at=B.Example:8443")).toEqual(
      at("b.example:8443"),
    );
    expect(parse("file:///aspen/index.html#/invite/abc123?at=b.example%3A8443")).toEqual(
      at("b.example:8443"),
    );
    expect(parse("https://a.example/at/b.example%3A8443/invite/abc123")).toEqual(
      at("b.example:8443"),
    );
    expect(parse("https://chat.example.org/at/b.example/invite/abc123/")).toEqual(at("b.example"));
    expect(parse("aspen://app/invite/abc123?at=b.example")).toEqual(at("b.example"));
    // The deployment in use's own link may name another.
    expect(parse("https://chat.example.org/invite/abc123?at=b.example")).toEqual(at("b.example"));
  });

  it("takes a link that names no deployment for the one whose address it is on", () => {
    expect(parse("https://other.example/invite/abc123")).toEqual({
      code: "abc123",
      domain: "other.example",
    });
    expect(parse("https://Other.Example:8443/invite/abc123?utm=1")).toEqual({
      code: "abc123",
      domain: "other.example:8443",
    });
    expect(parse("https://other.example/#/invite/abc123")).toEqual({
      code: "abc123",
      domain: "other.example",
    });
    // With no addresses known to be the deployment's own, every host is another's.
    expect(parseInvite("https://chat.example.org/invite/abc123", [])).toEqual({
      code: "abc123",
      domain: "chat.example.org",
    });
  });

  it("rejects anything that is not a code", () => {
    expect(parse("")).toBeNull();
    expect(parse("not a code")).toBeNull();
    expect(parse("https://chat.example.org/communities/x")).toBeNull();
    expect(parse("abcdefghijklmnopq")).toBeNull();
  });

  it("reads an invite only as a link's whole path", () => {
    for (const link of [
      // More of a path, before the route or after it.
      "https://chat.example.org/docs/invite/setup",
      "https://chat.example.org/invite/abc123/members",
      "https://chat.example.org/at/b.example/x/invite/abc123",
      "https://chat.example.org/at//invite/abc123",
      // The route in a query, in a fragment that is not a page's route, or in prose.
      "https://chat.example.org/search?q=/invite/abc123",
      "https://chat.example.org/?next=https://chat.example.org/invite/abc123",
      "https://chat.example.org/docs#/invite/abc123",
      "https://chat.example.org/docs#see-/invite/abc123",
      "join at https://chat.example.org/invite/abc123",
      "https://chat.example.org/invite/abc123 is the link",
      "/invite/abc123",
      "chat.example.org/invite/abc123",
      // Another scheme, or another host of the app's own.
      "ftp://chat.example.org/invite/abc123",
      "mailto:kate@chat.example.org?subject=/invite/abc123",
      "aspen://other/invite/abc123",
    ]) {
      expect(parse(link), link).toBeNull();
    }
  });

  it("holds a link's code to the rule a bare code is held to", () => {
    const longest = "abcdefghijklmnop";
    const tooLong = `${longest}q`;
    expect(parse(longest)?.code).toBe(longest);
    expect(parse(`https://a.example/invite/${longest}`)?.code).toBe(longest);
    expect(parse(`https://a.example/at/b.example/invite/${longest}`)?.code).toBe(longest);
    for (const code of [tooLong, "abc-123", "café"]) {
      expect(parse(code)).toBeNull();
      expect(parse(`https://a.example/invite/${code}`)).toBeNull();
      expect(parse(`https://a.example/at/b.example/invite/${code}`)).toBeNull();
    }
  });
});

describe("namedInvite", () => {
  it("is an invite link that names its deployment itself", () => {
    const at = { code: "abc123", domain: "b.example" };
    expect(namedInvite("https://a.example/invite/abc123?at=b.example")).toEqual(at);
    expect(namedInvite("https://a.example/at/b.example/invite/abc123")).toEqual(at);
    expect(namedInvite("aspen://app/invite/abc123?at=b.example")).toEqual(at);
  });

  it("is not a link that only has an invite's path, nor a bare code", () => {
    expect(namedInvite("https://discord.example/invite/abc123")).toBeNull();
    expect(namedInvite("https://a.example/invite/abc123?at=")).toBeNull();
    expect(namedInvite("https://a.example/watch?v=/invite/abc123&at=b.example")).toBeNull();
    expect(namedInvite("abc123")).toBeNull();
  });
});
