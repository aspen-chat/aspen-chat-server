import { describe, expect, it } from "vitest";
import { parseInviteCode } from "./inviteCode";

describe("parseInviteCode", () => {
  it("accepts bare codes and links from either history mode", () => {
    expect(parseInviteCode("  abc123 ")).toBe("abc123");
    expect(parseInviteCode("https://chat.example.org/invite/abc123")).toBe("abc123");
    expect(parseInviteCode("https://chat.example.org/invite/abc123/")).toBe("abc123");
    expect(parseInviteCode("file:///opt/aspen/index.html#/invite/abc123")).toBe("abc123");
    expect(parseInviteCode("http://localhost:5173/invite/abc123?utm=1")).toBe("abc123");
  });

  it("rejects anything that is not a code", () => {
    expect(parseInviteCode("")).toBeNull();
    expect(parseInviteCode("not a code")).toBeNull();
    expect(parseInviteCode("https://chat.example.org/communities/x")).toBeNull();
    expect(parseInviteCode("abcdefghijklmnopq")).toBeNull();
  });
});
