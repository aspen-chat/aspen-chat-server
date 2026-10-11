import { describe, expect, it } from "vitest";
import { foldThreads } from "@/features/activity/threads";

const entry = (key: string, id: string, channelId: string) => ({ key, message: { id, channelId } });
const inThread = (_key: string, channelId: string) => channelId.startsWith("thread");

describe("foldThreads", () => {
  it("keeps a thread's newest reply and leaves out its older ones", () => {
    const folded = foldThreads(
      [
        entry("", "5", "thread-a"),
        entry("", "4", "general"),
        entry("", "3", "thread-a"),
        entry("", "2", "general"),
        entry("", "1", "thread-b"),
      ],
      inThread,
    );
    expect(folded.map(({ message, thread }) => [message.id, thread])).toEqual([
      ["5", true],
      ["4", false],
      ["2", false],
      ["1", true],
    ]);
  });

  it("keeps threads of different deployments apart", () => {
    const folded = foldThreads(
      [entry("", "2", "thread-a"), entry("other.example", "1", "thread-a")],
      inThread,
    );
    expect(folded.map(({ message }) => message.id)).toEqual(["2", "1"]);
  });
});
