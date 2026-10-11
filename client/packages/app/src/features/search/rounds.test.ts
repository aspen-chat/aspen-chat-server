import type { Message, SearchPage } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { positionTime, searchUntilFound } from "./rounds";

const message = { id: "m" } as Message;
const empty = (searchedTo: string): SearchPage => ({ messages: [], searchedTo, more: true });

describe("searchUntilFound", () => {
  it("asks again from where an empty page read to, until a page holds something", async () => {
    const pages: SearchPage[] = [
      empty("p1"),
      empty("p2"),
      { messages: [message], searchedTo: "p3", more: true },
    ];
    const asked: (string | undefined)[] = [];
    const rounds: string[] = [];
    const page = await searchUntilFound(
      (before) => {
        asked.push(before);
        return Promise.resolve(pages.shift() ?? empty("never"));
      },
      undefined,
      (searchedTo) => rounds.push(searchedTo),
      new AbortController().signal,
    );
    expect(asked).toEqual([undefined, "p1", "p2"]);
    expect(rounds).toEqual(["p1", "p2"]);
    expect(page.messages).toEqual([message]);
  });

  it("stops at an empty page with nothing left", async () => {
    let asks = 0;
    const page = await searchUntilFound(
      () => {
        asks += 1;
        return Promise.resolve({ messages: [], searchedTo: null, more: false });
      },
      "start",
      () => undefined,
      new AbortController().signal,
    );
    expect(asks).toBe(1);
    expect(page.more).toBe(false);
  });

  it("asks no more once given up, and keeps where it had read to", async () => {
    const stop = new AbortController();
    let asks = 0;
    const page = await searchUntilFound(
      () => {
        asks += 1;
        stop.abort();
        return Promise.resolve(empty("p1"));
      },
      undefined,
      () => undefined,
      stop.signal,
    );
    expect(asks).toBe(1);
    expect(page).toEqual(empty("p1"));
  });
});

describe("positionTime", () => {
  it("reads the millisecond a UUIDv7 was made in", () => {
    expect(positionTime("019d04eb-1d73-7000-8000-000000000000")).toBe(0x019d04eb1d73);
  });
});
