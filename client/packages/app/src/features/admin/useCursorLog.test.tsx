// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { type CursorLog, type LogPage, useCursorLog } from "./useCursorLog";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/**
 * Mounts the hook over a log whose pages are `pages`, by the cursor that asks for each (the
 * first under ""), and gives what it shows now, the cursors asked for so far, and ways to
 * press Show more and to read it again.
 */
async function mount(pages: Record<string, LogPage<string>>) {
  const asked: (string | undefined)[] = [];
  const load = (cursor?: string) => {
    asked.push(cursor);
    const found = pages[cursor ?? ""];
    return found === undefined ? Promise.reject(new Error("no such page")) : Promise.resolve(found);
  };
  let shown: CursorLog<string> | undefined;
  function Log() {
    shown = useCursorLog(load);
    return null;
  }
  const root = createRoot(document.createElement("div"));
  await act(async () => {
    root.render(<Log />);
    await Promise.resolve();
  });
  const now = () => {
    if (shown === undefined) {
      throw new Error("not mounted");
    }
    return shown;
  };
  const press = (button: "showMore" | "reload") =>
    act(async () => {
      now()[button]();
      await Promise.resolve();
    });
  return { now, asked, showMore: () => press("showMore"), reload: () => press("reload") };
}

describe("useCursorLog", () => {
  it("asks for each page by the cursor the one before gave, and stops at the answer without one", async () => {
    const log = await mount({
      "": { data: ["a", "b"], nextCursor: "after-b" },
      "after-b": { data: ["c", "d"], nextCursor: null },
    });
    expect(log.now().entries).toEqual(["a", "b"]);
    expect(log.now().more).toBe(true);
    expect(log.now().loading).toBe(false);

    await log.showMore();
    expect(log.asked).toEqual([undefined, "after-b"]);
    expect(log.now().entries).toEqual(["a", "b", "c", "d"]);
    // As full as the page before, and the last all the same.
    expect(log.now().more).toBe(false);

    await log.showMore();
    expect(log.asked).toEqual([undefined, "after-b"]);
  });

  it("does not take a short page, or an empty one, with a cursor as the end", async () => {
    const log = await mount({
      "": { data: ["a", "b", "c"], nextCursor: "after-c" },
      "after-c": { data: ["d"], nextCursor: "after-d" },
      "after-d": { data: [], nextCursor: "after-nothing" },
      "after-nothing": { data: ["e"], nextCursor: null },
    });
    await log.showMore();
    expect(log.now().more).toBe(true);
    await log.showMore();
    expect(log.now().entries).toEqual(["a", "b", "c", "d"]);
    expect(log.now().more).toBe(true);
    await log.showMore();
    expect(log.asked).toEqual([undefined, "after-c", "after-d", "after-nothing"]);
    expect(log.now().entries).toEqual(["a", "b", "c", "d", "e"]);
    expect(log.now().more).toBe(false);
  });

  it("offers the same page again after a read of it fails, and starts over when read again", async () => {
    const pages: Record<string, LogPage<string>> = { "": { data: ["a"], nextCursor: "after-a" } };
    const log = await mount(pages);
    await log.showMore();
    expect(log.now().error).toBe("Error: no such page");
    expect(log.now().entries).toEqual(["a"]);
    expect(log.now().more).toBe(true);

    pages["after-a"] = { data: ["b"], nextCursor: null };
    await log.showMore();
    expect(log.asked).toEqual([undefined, "after-a", "after-a"]);
    expect(log.now().error).toBeNull();
    expect(log.now().entries).toEqual(["a", "b"]);

    await log.reload();
    expect(log.asked).toEqual([undefined, "after-a", "after-a", undefined]);
    expect(log.now().entries).toEqual(["a"]);
    expect(log.now().more).toBe(true);
  });
});
