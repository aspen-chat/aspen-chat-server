import { ApiProblemError, type Message, type MessagePage } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { withFeedPage } from "@/features/activity/feedPages";

const message = (id: string) => ({ id }) as MessagePage["data"][number];
const ids = (messages: readonly Message[]) => messages.map((m) => m.id);

/** A feed whose pages are `pages`, by the cursor that asks for each (the first under ""). */
function feed(pages: Record<string, MessagePage>) {
  const asked: (string | undefined)[] = [];
  const read = (cursor: string | undefined) => {
    asked.push(cursor);
    const found = pages[cursor ?? ""];
    return found === undefined ? Promise.reject(new Error("no such page")) : Promise.resolve(found);
  };
  return { asked, read };
}

describe("withFeedPage", () => {
  it("goes on by the cursor each page gives, and ends at the page without one", async () => {
    const { asked, read } = feed({
      "": { data: [message("5"), message("4")], nextCursor: "after-4" },
      // As full as the page before, and the last all the same.
      "after-4": { data: [message("3"), message("2")], nextCursor: null },
    });
    const first = await withFeedPage(read, [], undefined);
    expect(ids(first.messages)).toEqual(["5", "4"]);
    expect(first).toMatchObject({ next: "after-4", exhausted: false, problem: null });

    const second = await withFeedPage(read, first.messages, first.next ?? undefined);
    expect(ids(second.messages)).toEqual(["5", "4", "3", "2"]);
    expect(second).toMatchObject({ next: null, exhausted: true, problem: null });
    expect(asked).toEqual([undefined, "after-4"]);
  });

  it("does not take a short page, or an empty one, with a cursor as the end", async () => {
    const { read } = feed({
      "": { data: [message("5")], nextCursor: "after-5" },
      "after-5": { data: [], nextCursor: "after-nothing" },
    });
    const first = await withFeedPage(read, [], undefined);
    expect(first.exhausted).toBe(false);
    const second = await withFeedPage(read, first.messages, "after-5");
    expect(ids(second.messages)).toEqual(["5"]);
    expect(second).toMatchObject({ next: "after-nothing", exhausted: false });
  });

  it("keeps what was read and says why when a page cannot be read", async () => {
    const earlier = [message("5")];
    const failed = await withFeedPage(
      () =>
        Promise.reject(
          new ApiProblemError({ code: "serverBusy", title: "Busy", status: 503, detail: "Later." }),
        ),
      earlier,
      "after-5",
    );
    expect(failed.messages).toBe(earlier);
    expect(failed).toMatchObject({ next: null, exhausted: true });
    expect(failed.problem).not.toBeNull();
  });
});
