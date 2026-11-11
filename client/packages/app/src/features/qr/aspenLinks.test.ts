import { describe, expect, it } from "vitest";
import { deviceLinkPath, parseAspenLink as parse } from "./aspenLinks";

/** The addresses of the deployment in use. */
const HOME = ["chat.example.org"];
const parseAspenLink = (text: string) => parse(text, HOME);

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
    const registration = { kind: "registration", code: "Abc123", domain: null };
    expect(parseAspenLink("https://chat.example.org/register?invite=Abc123")).toEqual(registration);
    expect(parseAspenLink("aspen://app/register?invite=Abc123")).toEqual(registration);
    expect(parseAspenLink("file:///opt/aspen/index.html#/register?invite=Abc123")).toEqual(
      registration,
    );
    expect(parseAspenLink("aspen://app/invite/abc123?at=b.example")).toEqual({
      kind: "invite",
      invite: { code: "abc123", domain: "b.example" },
    });
    expect(parseAspenLink("https://chat.example.org/invite/abc123")).toEqual({
      kind: "invite",
      invite: { code: "abc123", domain: null },
    });
  });

  const community = "01a1092e-b636-735b-8ae8-356fceb84f99";
  const channel = "01a1092e-b683-700d-8d11-d5cad1e2e76b";

  it("reads channels and DMs, as mail links to them", () => {
    const here = { kind: "channel", domain: null, community, channel };
    expect(
      parseAspenLink(`https://chat.example.org/communities/${community}/channels/${channel}`),
    ).toEqual(here);
    expect(parseAspenLink(`aspen://app/communities/${community}/channels/${channel}`)).toEqual(
      here,
    );
    expect(parseAspenLink(`aspen://app/dms/${channel}`)).toEqual({
      kind: "dm",
      domain: null,
      channel,
    });
    expect(parseAspenLink("aspen://app/dms/not-an-id")).toBeNull();
  });

  it("takes a link on another deployment's address, or under /at/, for that deployment's", () => {
    expect(parseAspenLink("https://b.example/register?invite=Abc123")).toEqual({
      kind: "registration",
      code: "Abc123",
      domain: "b.example",
    });
    expect(parseAspenLink("https://b.example/invite/abc123")).toEqual({
      kind: "invite",
      invite: { code: "abc123", domain: "b.example" },
    });
    expect(
      parseAspenLink(`https://B.example:8443/communities/${community}/channels/${channel}`),
    ).toEqual({ kind: "channel", domain: "b.example:8443", community, channel });
    const abroad = { kind: "channel", domain: "c.example", community, channel };
    for (const base of ["https://chat.example.org", "https://b.example", "aspen://app"]) {
      expect(
        parseAspenLink(`${base}/at/c.example/communities/${community}/channels/${channel}`),
      ).toEqual(abroad);
    }
    expect(parseAspenLink(`aspen://app/at/c.example%3A8443/dms/${channel}`)).toEqual({
      kind: "dm",
      domain: "c.example:8443",
      channel,
    });
    // An account is made at a deployment's own address.
    expect(parseAspenLink("https://chat.example.org/at/c.example/register?invite=Abc")).toBeNull();
  });

  it("reads a route only as a link's whole path", () => {
    const place = `/communities/${community}/channels/${channel}`;
    for (const link of [
      // More of a path, before the route or after it.
      "https://shop.example/promo/register?invite=SALE",
      "https://chat.example.org/register/now?invite=Abc123",
      `https://chat.example.org/x${place}`,
      `https://chat.example.org${place}/extra`,
      `https://chat.example.org/x/dms/${channel}`,
      "https://chat.example.org/x/device-link?server=https%3A%2F%2Fapi.example.org#" + ID,
      // The route in a query, in a fragment that is not a page's route, or in prose.
      `https://chat.example.org/search?q=/register?invite=Abc123`,
      `https://chat.example.org/search?next=${place}`,
      `https://chat.example.org/docs#${place}`,
      `https://chat.example.org/docs#/register?invite=Abc123`,
      `open https://chat.example.org${place}`,
      `https://chat.example.org/register?invite=Abc123 to sign up`,
      "/register?invite=Abc123",
      // Another scheme.
      "ftp://chat.example.org/register?invite=Abc123",
    ]) {
      expect(parseAspenLink(link), link).toBeNull();
    }
    // A channel link whose query holds another route is still the channel.
    expect(parseAspenLink(`https://chat.example.org${place}?from=/register?invite=Abc`)).toEqual({
      kind: "channel",
      domain: null,
      community,
      channel,
    });
  });

  it("refuses anything else", () => {
    expect(parseAspenLink("abc123")).toBeNull();
    expect(parseAspenLink("https://example.org/")).toBeNull();
    expect(parseAspenLink("otpauth://totp/Aspen:kate?secret=X")).toBeNull();
  });
});
