import { describe, expect, it } from "vitest";
import { unreadEntries } from "./unread";

const reply = (id: string, channelId: string) => ({ id, channelId, timestamp: "" });

describe("the Unread tab's entries", () => {
  it("orders channels and threads together by their newest unread message", () => {
    const entries = unreadEntries(
      [
        { key: "", channelId: "general", newest: "3" },
        { key: "far", channelId: "lobby", newest: "5" },
      ],
      [{ key: "", exhausted: true, messages: [reply("4", "thread")] }],
    );
    expect(entries.map((e) => e.channelId)).toEqual(["lobby", "thread", "general"]);
    expect(entries.map((e) => e.thread)).toEqual([false, true, false]);
  });

  it("shows a thread once, where its newest reply stands", () => {
    const entries = unreadEntries(
      [],
      [{ key: "", exhausted: true, messages: [reply("9", "t"), reply("7", "u"), reply("6", "t")] }],
    );
    expect(entries).toEqual([
      { key: "", channelId: "t", newest: "9", thread: true },
      { key: "", channelId: "u", newest: "7", thread: true },
    ]);
  });

  it("keeps the same thread id apart on two deployments", () => {
    const entries = unreadEntries(
      [],
      [
        { key: "", exhausted: true, messages: [reply("2", "t")] },
        { key: "far", exhausted: true, messages: [reply("1", "t")] },
      ],
    );
    expect(entries.map((e) => e.key)).toEqual(["", "far"]);
  });

  it("holds back what is older than a deployment's replies still to be read", () => {
    const entries = unreadEntries(
      [
        { key: "", channelId: "new", newest: "8" },
        { key: "", channelId: "old", newest: "2" },
      ],
      [
        { key: "", exhausted: false, messages: [reply("6", "t")] },
        { key: "far", exhausted: true, messages: [reply("3", "u")] },
      ],
    );
    expect(entries.map((e) => e.channelId)).toEqual(["new", "t"]);
  });
});
