import { describe, expect, it } from "vitest";
import { parseSelfLink, type KnownHosts } from "@/features/messages/selfLinks";

const c = "01a0da3e-2ad1-71c3-9f7a-c8e1c917695f";
const ch = "01a0da3e-2ad6-7371-aa93-f00b2c6d541e";
const msg = "01a0dbf0-e813-7371-9e7c-86ae12110432";

const hosts: KnownHosts = { home: ["chat.example", "localhost:5173"], foreign: ["other.example"] };
const parse = (href: string) => parseSelfLink(href, hosts);

describe("links to a deployment the user uses", () => {
  it("name the home's routes on its addresses", () => {
    expect(parse("https://chat.example")).toEqual({ domain: null, route: { kind: "deployment" } });
    expect(parse(`https://chat.example/communities/${c}/`)).toEqual({
      domain: null,
      route: { kind: "community", community: c },
    });
    expect(parse(`http://localhost:5173/communities/${c}/channels/${ch}`)).toEqual({
      domain: null,
      route: { kind: "channel", community: c, channel: ch },
    });
    expect(parse(`https://chat.example/communities/${c}/channels/${ch}/messages/${msg}`)).toEqual({
      domain: null,
      route: { kind: "message", community: c, channel: ch, message: msg },
    });
    expect(parse(`https://chat.example/dms/${ch}/threads/${msg}`)).toEqual({
      domain: null,
      route: { kind: "thread", community: null, channel: ch, thread: msg },
    });
    expect(parse("https://chat.example/dms")).toEqual({ domain: null, route: { kind: "dms" } });
    expect(parse("https://chat.example/invite/abc123")).toEqual({
      domain: null,
      route: { kind: "invite", code: "abc123" },
    });
  });

  it("name the home's own pages", () => {
    expect(parse("https://chat.example/admin")?.route).toEqual({ kind: "admin", tab: null });
    expect(parse("https://chat.example/admin/reports")?.route).toEqual({
      kind: "admin",
      tab: "reports",
    });
    expect(parse(`https://chat.example/bots/${c}/add?permissions=sendMessages`)?.route).toEqual({
      kind: "botAdd",
      bot: c,
      permissions: "sendMessages",
    });
    expect(parse("https://chat.example/register?invite=XYZ")?.route).toEqual({
      kind: "registration",
      code: "XYZ",
    });
    expect(parse("https://chat.example/attributions")?.route).toEqual({ kind: "attributions" });
    const id = "a".repeat(43);
    expect(
      parse(`https://chat.example/device-link?server=https%3A%2F%2Fchat.example#${id}`)?.route,
    ).toEqual({ kind: "deviceLink", server: "https://chat.example", id });
  });

  it("name another deployment's routes, on its address or the home's", () => {
    expect(parse(`https://other.example/communities/${c}`)).toEqual({
      domain: "other.example",
      route: { kind: "community", community: c },
    });
    expect(parse(`https://chat.example/at/other.example/dms/${ch}`)).toEqual({
      domain: "other.example",
      route: { kind: "channel", community: null, channel: ch },
    });
    expect(parse(`https://chat.example/at/chat.example/communities/${c}`)).toEqual({
      domain: null,
      route: { kind: "community", community: c },
    });
    // Its own pages are its own business: there is no route here for them.
    expect(parse("https://other.example/admin")).toBeNull();
  });

  it("follow an invite's ?at= to the deployment it names", () => {
    expect(parse("https://chat.example/invite/abc?at=other.example")).toEqual({
      domain: "other.example",
      route: { kind: "invite", code: "abc" },
    });
    expect(parse("https://other.example/invite/abc?at=chat.example")?.domain).toBeNull();
    expect(parse("https://chat.example/invite/abc?at=elsewhere.example")).toBeNull();
  });

  it("leave every other link alone", () => {
    expect(parse(`https://elsewhere.example/communities/${c}`)).toBeNull();
    expect(parse(`https://chat.example/at/elsewhere.example/communities/${c}`)).toBeNull();
    expect(parse("https://chat.example/api/v1/users/@me")).toBeNull();
    expect(parse("https://chat.example/communities/not-an-id")).toBeNull();
    expect(parse("https://chat.example/.well-known/aspen")).toBeNull();
    expect(parse(`ftp://chat.example/communities/${c}`)).toBeNull();
    expect(parse("mailto:someone@chat.example")).toBeNull();
    expect(parse("not a url")).toBeNull();
  });
});
