import { describe, expect, it } from "vitest";
import { wantsNativeMenu, type ContextMenuCause } from "@/features/layout/contextMenuState";

/** A right click on `target`, inside `currentTarget`, with the mouse and nothing held. */
function click(target: Element, currentTarget: Element, rest: Partial<ContextMenuCause> = {}) {
  return {
    shiftKey: false,
    defaultPrevented: false,
    target,
    currentTarget,
    pointerType: "mouse",
    ...rest,
  };
}

describe("wantsNativeMenu", () => {
  const row = document.createElement("div");
  const text = document.createElement("p");
  text.textContent = "Sounds good";
  const field = document.createElement("textarea");
  const video = document.createElement("video");
  row.append(text, field, video);
  document.body.append(row);
  const none = window.getSelection();
  none?.removeAllRanges();

  it("leaves a plain right click to the app", () => {
    expect(wantsNativeMenu(click(text, row), none)).toBe(false);
  });

  it("gives the browser Shift, a finger, and what something inside already took", () => {
    expect(wantsNativeMenu(click(text, row, { shiftKey: true }), none)).toBe(true);
    expect(wantsNativeMenu(click(text, row, { pointerType: "touch" }), none)).toBe(true);
    expect(wantsNativeMenu(click(text, row, { defaultPrevented: true }), none)).toBe(true);
  });

  it("gives the browser text fields and playing media", () => {
    expect(wantsNativeMenu(click(field, row), none)).toBe(true);
    expect(wantsNativeMenu(click(video, row), none)).toBe(true);
  });

  it("gives the browser text selected inside what was clicked, and nowhere else", () => {
    const selection = window.getSelection();
    if (selection === null) {
      throw new Error("no selection in this document");
    }
    selection.selectAllChildren(text);
    expect(wantsNativeMenu(click(text, row), selection)).toBe(true);
    const elsewhere = document.createElement("div");
    document.body.append(elsewhere);
    expect(wantsNativeMenu(click(elsewhere, elsewhere), selection)).toBe(false);
    selection.removeAllRanges();
  });
});
