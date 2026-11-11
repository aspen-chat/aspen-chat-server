// @vitest-environment jsdom
import type { Channel } from "@aspen/protocol";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import type { LogPage } from "@/features/admin/useCursorLog";
import { UserDmList } from "./UserDmList";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// A row is a link named for the DM; neither the router nor the store is under test.
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, params }: { children: ReactNode; params: { channelId: string } }) => (
    <a href={`/dms/${params.channelId}`}>{children}</a>
  ),
}));
vi.mock("@/features/dms/useDmTitle", () => ({
  useDmTitle: (dm: Channel) => `DM ${dm.id}`,
}));

const page = (ids: readonly string[], nextCursor: string | null): LogPage<Channel> => ({
  data: ids.map((id) => ({ id }) as Channel),
  nextCursor,
});

/**
 * Mounts the list over DMs whose pages are `pages`, by the cursor that asks for each (the first
 * under ""), and gives the DMs it links to, the cursors asked for, and its Show more button.
 */
async function mount(pages: Record<string, LogPage<Channel>>) {
  const asked: (string | undefined)[] = [];
  const load = (cursor?: string) => {
    asked.push(cursor);
    const found = pages[cursor ?? ""];
    return found === undefined ? Promise.reject(new Error("no such page")) : Promise.resolve(found);
  };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  await act(async () => {
    root.render(<UserDmList load={load} />);
    await Promise.resolve();
  });
  const showMore = () =>
    [...container.querySelectorAll("button")].find((b) => b.textContent === "Show more");
  return {
    asked,
    container,
    links: () => [...container.querySelectorAll("a")].map((a) => a.textContent),
    showMore,
    pressShowMore: () =>
      act(async () => {
        showMore()?.click();
        await Promise.resolve();
      }),
  };
}

describe("UserDmList", () => {
  it("shows more DMs by the cursor the page before gave, until a page gives none", async () => {
    const list = await mount({
      "": page(["a", "b"], "after-b"),
      "after-b": page(["c", "d"], null),
    });
    expect(list.links()).toEqual(["DM a", "DM b"]);
    expect(list.showMore()).toBeDefined();

    await list.pressShowMore();
    expect(list.asked).toEqual([undefined, "after-b"]);
    expect(list.links()).toEqual(["DM a", "DM b", "DM c", "DM d"]);
    // As full as the page before, and the last all the same.
    expect(list.showMore()).toBeUndefined();
  });

  it("offers no more after a first page that is the last, and says when there are none", async () => {
    const few = await mount({ "": page(["a"], null) });
    expect(few.links()).toEqual(["DM a"]);
    expect(few.showMore()).toBeUndefined();

    const none = await mount({ "": page([], null) });
    expect(none.links()).toEqual([]);
    expect(none.container.textContent).toContain("No DMs.");
    expect(none.showMore()).toBeUndefined();
  });

  it("keeps the DMs read and says why when the next page cannot be read", async () => {
    const list = await mount({ "": page(["a"], "after-a") });
    await list.pressShowMore();
    expect(list.links()).toEqual(["DM a"]);
    expect(list.container.querySelector('[role="alert"]')?.textContent).toBe("Error: no such page");
    // The same page can be asked for again.
    expect(list.showMore()).toBeDefined();
  });
});
