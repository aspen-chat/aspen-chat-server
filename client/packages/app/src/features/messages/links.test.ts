import { describe, expect, it } from "vitest";
import { messageUrl } from "@/features/messages/links";
import { parseSelfLink } from "@/features/messages/selfLinks";

const c = "01a0da3e-2ad1-71c3-9f7a-c8e1c917695f";
const ch = "01a0da3e-2ad6-7371-aa93-f00b2c6d541e";
const msg = "01a0dbf0-e813-7371-9e7c-86ae12110432";

describe("message links", () => {
  it("are read back as they are made, in a community and in a DM", () => {
    const hosts = { home: ["chat.example", "localhost:8000"], foreign: [] };
    expect(parseSelfLink(messageUrl("https://chat.example", c, ch, msg), hosts)).toEqual({
      domain: null,
      route: { kind: "message", community: c, channel: ch, message: msg },
    });
    expect(parseSelfLink(messageUrl("http://localhost:8000/", null, ch, msg), hosts)).toEqual({
      domain: null,
      route: { kind: "message", community: null, channel: ch, message: msg },
    });
  });
});
