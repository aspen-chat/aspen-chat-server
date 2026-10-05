import type { KeyboardEvent as ReactKeyboardEvent } from "react";
import { describe, expect, it } from "vitest";
import { MessageRows, moveBetweenRows } from "./messageRows";

function list(count: number): HTMLElement {
  const box = document.createElement("div");
  for (let i = 0; i < count; i++) {
    const row = document.createElement("article");
    row.dataset.messageRow = `m${String(i)}`;
    row.tabIndex = -1;
    row.appendChild(document.createElement("button"));
    box.appendChild(row);
  }
  document.body.replaceChildren(box);
  return box;
}

/** The `index`th row of `box`. */
function rowOf(box: HTMLElement, index: number): HTMLElement {
  const row = box.children[index];
  if (!(row instanceof HTMLElement)) {
    throw new Error(`no row ${String(index)}`);
  }
  return row;
}

function press(target: EventTarget, key: string, modifiers: Partial<KeyboardEvent> = {}) {
  let prevented = false;
  const event = {
    key,
    target,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    ...modifiers,
    preventDefault: () => {
      prevented = true;
    },
  } as unknown as ReactKeyboardEvent;
  return { event, prevented: () => prevented };
}

describe("moveBetweenRows", () => {
  it("moves focus to the row before, after, first, and last", () => {
    const box = list(4);
    const up = press(rowOf(box, 2), "ArrowUp");
    expect(moveBetweenRows(box, up.event)).toBe("older");
    expect(up.prevented()).toBe(true);
    expect(document.activeElement).toBe(rowOf(box, 1));
    expect(moveBetweenRows(box, press(rowOf(box, 1), "End").event)).toBe("newer");
    expect(document.activeElement).toBe(rowOf(box, 3));
    expect(moveBetweenRows(box, press(rowOf(box, 3), "Home").event)).toBe("older");
    expect(document.activeElement).toBe(rowOf(box, 0));
  });

  it("leaves keys on a row's controls, and with modifiers, alone", () => {
    const box = list(3);
    const onControl = press(rowOf(box, 0).firstElementChild ?? box, "ArrowDown");
    expect(moveBetweenRows(box, onControl.event)).toBeNull();
    expect(onControl.prevented()).toBe(false);
    const row = rowOf(box, 1);
    expect(moveBetweenRows(box, press(row, "ArrowDown", { shiftKey: true }).event)).toBeNull();
    expect(moveBetweenRows(box, press(row, "a").event)).toBeNull();
  });
});

describe("MessageRows", () => {
  it("stops at the newest row until one is focused, while it is drawn", () => {
    const box = list(3);
    const rows = new MessageRows();
    rows.settle(box);
    expect(rows.stop).toBe("m2");
    rows.focused("m0");
    rows.settle(box);
    expect(rows.stop).toBe("m0");
    rowOf(box, 0).remove();
    rows.settle(box);
    expect(rows.stop).toBe("m2");
  });
});
