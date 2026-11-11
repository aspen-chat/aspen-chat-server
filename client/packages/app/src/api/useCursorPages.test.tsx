// @vitest-environment jsdom
import type { User, UserPage } from "@aspen/protocol";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { type CursorPages, useCursorPages } from "./useCursorPages";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** A page of the people named, of whom the hook keeps only the ids. */
function page(ids: readonly string[], nextCursor: string | null): UserPage {
  return { data: ids.map((id) => ({ id }) as User), nextCursor };
}

/**
 * Mounts the hook over a list whose pages are `pages`, by the cursor that asks for each (the
 * first under ""), and gives what it shows now, the cursors asked for so far, and a way to
 * press Show more.
 */
async function mount(pages: Record<string, UserPage>) {
  const asked: (string | undefined)[] = [];
  const load = (cursor?: string) => {
    asked.push(cursor);
    const found = pages[cursor ?? ""];
    return found === undefined ? Promise.reject(new Error("no such page")) : Promise.resolve(found);
  };
  let shown: CursorPages | undefined;
  function List() {
    shown = useCursorPages(load);
    return null;
  }
  const root = createRoot(document.createElement("div"));
  await act(async () => {
    root.render(<List />);
    await Promise.resolve();
  });
  const now = () => {
    if (shown === undefined) {
      throw new Error("not mounted");
    }
    return shown;
  };
  return {
    now,
    asked,
    showMore: () =>
      act(async () => {
        now().showMore();
        await Promise.resolve();
      }),
  };
}

describe("useCursorPages", () => {
  it("asks for each page by the cursor the one before gave, and stops at the answer without one", async () => {
    const list = await mount({
      "": page(["a", "b"], "after-b"),
      "after-b": page(["c", "d"], null),
    });
    expect(list.now().ids).toEqual(["a", "b"]);
    expect(list.now().more).toBe(true);
    expect(list.now().loading).toBe(false);

    await list.showMore();
    expect(list.asked).toEqual([undefined, "after-b"]);
    expect(list.now().ids).toEqual(["a", "b", "c", "d"]);
    // As full as the page before, and the last all the same.
    expect(list.now().more).toBe(false);

    await list.showMore();
    expect(list.asked).toEqual([undefined, "after-b"]);
  });

  it("does not take a short page, or an empty one, with a cursor as the end", async () => {
    const list = await mount({
      "": page(["a", "b", "c"], "after-c"),
      "after-c": page(["d"], "after-d"),
      "after-d": page([], "after-nobody"),
      "after-nobody": page(["e"], null),
    });
    await list.showMore();
    expect(list.now().ids).toEqual(["a", "b", "c", "d"]);
    expect(list.now().more).toBe(true);
    await list.showMore();
    expect(list.now().ids).toEqual(["a", "b", "c", "d"]);
    expect(list.now().more).toBe(true);
    await list.showMore();
    expect(list.asked).toEqual([undefined, "after-c", "after-d", "after-nobody"]);
    expect(list.now().ids).toEqual(["a", "b", "c", "d", "e"]);
    expect(list.now().more).toBe(false);
  });

  it("keeps its cursor past someone dropped here, and lists nobody twice", async () => {
    const list = await mount({
      "": page(["a", "b"], "after-b"),
      "after-b": page(["b", "c"], null),
    });
    // The last one read is taken out of the list; the next page is still asked for by the
    // cursor, which names no one.
    act(() => {
      list.now().drop("b");
    });
    expect(list.now().ids).toEqual(["a"]);
    expect(list.now().more).toBe(true);
    await list.showMore();
    expect(list.asked).toEqual([undefined, "after-b"]);
    expect(list.now().ids).toEqual(["a", "b", "c"]);
  });

  it("offers the same page again after a read of it fails", async () => {
    const pages: Record<string, UserPage> = { "": page(["a"], "after-a") };
    const list = await mount(pages);
    await list.showMore();
    expect(list.now().error).toBe("Error: no such page");
    expect(list.now().ids).toEqual(["a"]);
    expect(list.now().more).toBe(true);

    pages["after-a"] = page(["b"], null);
    await list.showMore();
    expect(list.asked).toEqual([undefined, "after-a", "after-a"]);
    expect(list.now().error).toBeNull();
    expect(list.now().ids).toEqual(["a", "b"]);
    expect(list.now().more).toBe(false);
  });
});
