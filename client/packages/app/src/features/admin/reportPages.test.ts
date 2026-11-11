import type { ReportCase, ReportCaseList } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { readCasePages } from "./reportPages";

/** A page of the cases named, naming nothing else. */
function page(ids: readonly string[], nextCursor: string | null): ReportCaseList {
  return {
    cases: ids.map((id) => ({ id }) as ReportCase),
    users: [],
    messages: [],
    channels: [],
    communities: [],
    categories: [],
    attachments: [],
    nextCursor,
  };
}

/** A list whose pages are `pages`, by the cursor that asks for each (the first under ""). */
function list(pages: Record<string, ReportCaseList>) {
  const asked: (string | undefined)[] = [];
  const read = (cursor?: string) => {
    asked.push(cursor);
    const found = pages[cursor ?? ""];
    return found === undefined ? Promise.reject(new Error("no such page")) : Promise.resolve(found);
  };
  return { asked, read };
}

describe("readCasePages", () => {
  it("asks for each page by the cursor the one before gave", async () => {
    const { asked, read } = list({
      "": page(["a"], "after-a"),
      "after-a": page(["b"], "after-b"),
      "after-b": page(["c"], null),
    });
    const two = await readCasePages(read, 2);
    expect(two.cases.map((c) => c.id)).toEqual(["a", "b"]);
    // Each page held one case where fifteen were asked for, and a third follows.
    expect(two.nextCursor).toBe("after-b");
    expect(asked).toEqual([undefined, "after-a"]);
  });

  it("stops at the answer without a cursor, however many pages were wanted", async () => {
    const { asked, read } = list({
      "": page(["a", "b"], "after-b"),
      "after-b": page(["c", "d"], null),
    });
    const all = await readCasePages(read, 5);
    expect(all.cases.map((c) => c.id)).toEqual(["a", "b", "c", "d"]);
    expect(all.nextCursor).toBeNull();
    expect(asked).toEqual([undefined, "after-b"]);
  });

  it("reads on past an empty page that gives a cursor", async () => {
    const { read } = list({
      "": page([], "after-nothing"),
      "after-nothing": page(["a"], null),
    });
    const all = await readCasePages(read, 2);
    expect(all.cases.map((c) => c.id)).toEqual(["a"]);
    expect(all.nextCursor).toBeNull();
  });
});
