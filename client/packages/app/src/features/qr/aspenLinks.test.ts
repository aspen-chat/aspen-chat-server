import { describe, expect, it } from "vitest";
import { deviceLinkPath, parseAspenLink } from "./aspenLinks";

const ID = "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789_-abcde";

describe("parseAspenLink", () => {
  it("reads a sign-in code under any address", () => {
    const path = deviceLinkPath("https://api.example.org", ID);
    const expected = { kind: "deviceLink", server: "https://api.example.org", id: ID };
    expect(parseAspenLink(`https://chat.example.org${path}`)).toEqual(expected);
    expect(parseAspenLink(`aspen://app${path}`)).toEqual(expected);
    expect(parseAspenLink(`file:///opt/aspen/index.html#${path}`)).toEqual(expected);
    expect(
      parseAspenLink(
        `https://localhost/#/device-link?server=https%3A%2F%2Fapi.example.org&link=${ID}`,
      ),
    ).toEqual(expected);
  });

  it("refuses a sign-in code without a server or a proper id", () => {
    expect(parseAspenLink(`https://chat.example.org/device-link#${ID}`)).toBeNull();
    expect(parseAspenLink("https://chat.example.org/device-link?server=x.org#short")).toBeNull();
    expect(
      parseAspenLink(`https://chat.example.org/device-link?server=ftp%3A%2F%2Fx.org#${ID}`),
    ).toBeNull();
  });

  it("reads registration and community invites", () => {
    expect(parseAspenLink("https://chat.example.org/register?invite=Abc123")).toEqual({
      kind: "registration",
      code: "Abc123",
    });
    expect(parseAspenLink("aspen://app/invite/abc123?at=b.example")).toEqual({
      kind: "invite",
      invite: { code: "abc123", domain: "b.example" },
    });
  });

  it("reads channels and DMs, as mail links to them", () => {
    const community = "01a1092e-b636-735b-8ae8-356fceb84f99";
    const channel = "01a1092e-b683-700d-8d11-d5cad1e2e76b";
    expect(
      parseAspenLink(`https://chat.example.org/communities/${community}/channels/${channel}`),
    ).toEqual({ kind: "channel", community, channel });
    expect(parseAspenLink(`aspen://app/communities/${community}/channels/${channel}`)).toEqual({
      kind: "channel",
      community,
      channel,
    });
    expect(parseAspenLink(`aspen://app/dms/${channel}`)).toEqual({ kind: "dm", channel });
    expect(parseAspenLink("aspen://app/dms/not-an-id")).toBeNull();
  });

  it("refuses anything else", () => {
    expect(parseAspenLink("abc123")).toBeNull();
    expect(parseAspenLink("https://example.org/")).toBeNull();
    expect(parseAspenLink("otpauth://totp/Aspen:kate?secret=X")).toBeNull();
  });
});
