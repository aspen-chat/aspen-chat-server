import type { AspenSync, Message } from "@aspen/protocol";
import { describe as group, expect, it } from "vitest";
import { en } from "@/i18n/messages";
import { describe, held } from "./describe";

const ANN = "0190a1b2-c3d4-7e5f-8a9b-000000000001";
const BOB = "0190a1b2-c3d4-7e5f-8a9b-000000000002";
const MODS = "0190a1b2-c3d4-7e5f-8a9b-000000000003";
const PARTY = "0190a1b2-c3d4-7e5f-8a9b-000000000004";
const ELSEWHERE = "0190a1b2-c3d4-7e5f-8a9b-000000000005";

/** A deployment holding one community, its channel, two people, a role, and an emoji. */
const sync = {
  store: {
    user: (id: string) =>
      id === ANN
        ? { id, name: "ann", displayName: "Ann" }
        : id === BOB
          ? { id, name: "bob" }
          : undefined,
    channel: (id: string) =>
      id === "general" ? { id, name: "general", community: "woods" } : undefined,
    community: (id: string) => (id === "woods" ? { id, name: "Woods" } : undefined),
    nickname: () => undefined,
    roles: () => [{ id: MODS, name: "Mods" }],
    customEmoji: (community: string) =>
      community === "woods" ? [{ id: PARTY, name: "party", community }] : [],
    attachment: () => undefined,
    poll: () => undefined,
  },
} as unknown as AspenSync;

function message(content: string): Message {
  return {
    id: "m",
    channelId: "general",
    author: ANN,
    content,
    kind: "standard",
    attachments: [],
  } as unknown as Message;
}

const body = (content: string) => describe(en, sync, message(content)).body;

group("describe", () => {
  it("names who wrote a message and where", () => {
    expect(describe(en, sync, message("hi"))).toEqual({
      name: "Ann",
      title: "Ann in #general (Woods)",
      body: "hi",
    });
  });

  it("writes tags and the community's emoji as names", () => {
    expect(body(`hi <@${BOB}> and <@&${MODS}> <:${PARTY}>`)).toBe("hi @bob and @Mods :party:");
  });

  it("leaves an emoji of another community as it was sent", () => {
    expect(body(`<:${ELSEWHERE}>`)).toBe(`<:${ELSEWHERE}>`);
  });

  it("writes each spoiler as the app's word for one, with what it holds", () => {
    expect(body(`the killer is ||<@${BOB}> <:${PARTY}>|| and >!the cook!<`)).toBe(
      "the killer is [spoiler] and [spoiler]",
    );
    expect(body("`||code||` ||a `b` c||")).toBe("`||code||` [spoiler]");
  });

  it("takes text that is all spoilers for text", () => {
    expect(body("||secret||")).toBe("[spoiler]");
  });

  it("says what a message with no text holds", () => {
    expect(body(" ")).toBe("New message");
  });
});

group("held", () => {
  it("says what a message of files alone holds", () => {
    expect(held(en, [])).toBe("New message");
    expect(held(en, ["image/png"])).toBe("Sent an image");
    expect(held(en, ["image/png", "Image/JPEG"])).toBe("Sent images");
    expect(held(en, ["application/pdf"])).toBe("Sent an attachment");
    expect(held(en, ["image/png", "video/mp4"])).toBe("Sent an attachment");
  });

  it("takes a file it does not hold for one that is not a picture", () => {
    expect(held(en, ["image/png", undefined])).toBe("Sent an attachment");
  });
});
