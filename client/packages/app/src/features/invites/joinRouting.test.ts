import { describe, expect, it } from "vitest";
import { parseAspenLink, type AspenLink } from "@/features/qr/aspenLinks";
import { routeJoinLink } from "./joinRouting";

const HOME = ["chat.example.org"];

function read(text: string): AspenLink {
  const link = parseAspenLink(text, HOME);
  if (link === null) {
    throw new Error(`not a link: ${text}`);
  }
  return link;
}

describe("routeJoinLink", () => {
  it("opens a community invite, of the deployment in use or another", () => {
    for (const text of [
      "https://chat.example.org/invite/abc",
      "https://other.example/invite/abc",
      "aspen://app/invite/abc",
    ]) {
      const link = read(text);
      expect(routeJoinLink(link, true)).toEqual({ action: "open", link });
      expect(routeJoinLink(link, false)).toEqual({ action: "open", link });
    }
  });

  it("opens a registration invite of the deployment in use", () => {
    for (const text of [
      "https://chat.example.org/register?invite=Welcome1",
      "aspen://app/register?invite=Welcome1",
    ]) {
      const link = read(text);
      expect(link).toEqual({ kind: "registration", code: "Welcome1", domain: null });
      expect(routeJoinLink(link, true)).toEqual({ action: "open", link });
    }
  });

  it("hands another deployment's registration invite to the add-a-server form", () => {
    expect(routeJoinLink(read("https://other.example/register?invite=Welcome1"), true)).toEqual({
      action: "otherServer",
      prefill: { domain: "other.example", registrationInvite: "Welcome1" },
    });
    expect(
      routeJoinLink(read("https://other.example:8443/register?invite=Welcome1"), true),
    ).toEqual({
      action: "otherServer",
      prefill: { domain: "other.example:8443", registrationInvite: "Welcome1" },
    });
  });

  it("refuses it, naming the deployment, where the home's accounts sign in nowhere else", () => {
    expect(routeJoinLink(read("https://other.example/register?invite=Welcome1"), false)).toEqual({
      action: "refuse",
      reason: "registrationElsewhere",
      domain: "other.example",
    });
  });

  it("refuses what is not an invite", () => {
    const refused = { action: "refuse", reason: "notAnInvite" };
    const id = "0199a0c0-0000-7000-8000-000000000001";
    for (const text of [
      `https://chat.example.org/device-link?server=${encodeURIComponent("https://chat.example.org")}#${"a".repeat(43)}`,
      `https://other.example/dms/${id}`,
      `https://chat.example.org/communities/${id}/channels/${id}`,
    ]) {
      expect(routeJoinLink(read(text), true)).toEqual(refused);
    }
  });
});
