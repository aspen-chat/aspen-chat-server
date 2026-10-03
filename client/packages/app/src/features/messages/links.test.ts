import { describe, expect, it } from "vitest";
import { messageUrl, parseMessageUrl } from "@/features/messages/links";

const c = "01a0da3e-2ad1-71c3-9f7a-c8e1c917695f";
const ch = "01a0da3e-2ad6-7371-aa93-f00b2c6d541e";
const msg = "01a0dbf0-e813-7371-9e7c-86ae12110432";

describe("message links", () => {
  it("are read back as they are made, in a community and in a DM", () => {
    expect(parseMessageUrl(messageUrl("https://chat.example", c, ch, msg))).toEqual({
      host: "chat.example",
      community: c,
      channel: ch,
      message: msg,
    });
    expect(parseMessageUrl(messageUrl("http://localhost:8000/", null, ch, msg))).toEqual({
      host: "localhost:8000",
      community: null,
      channel: ch,
      message: msg,
    });
  });

  it("leave other links alone, another deployment's routes included", () => {
    expect(parseMessageUrl(`https://example.com/messages/${msg}`)).toBeNull();
    expect(
      parseMessageUrl(`https://chat.example/at/other.example/dms/${ch}/messages/${msg}`),
    ).toBeNull();
    expect(parseMessageUrl(`ftp://chat.example/dms/${ch}/messages/${msg}`)).toBeNull();
    expect(parseMessageUrl("not a url")).toBeNull();
  });
});
