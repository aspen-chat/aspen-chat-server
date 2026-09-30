import { describe, expect, it } from "vitest";
import { isFullScreenKey, orientationFor } from "./fullScreen";

/** A keydown pressed on `target`, as it reaches the window. */
function press(init: KeyboardEventInit, target: Element = document.body): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { bubbles: true, ...init });
  target.dispatchEvent(event);
  return event;
}

function inside(html: string, selector: string): Element {
  document.body.innerHTML = html;
  const found = document.querySelector(selector);
  if (found === null) {
    throw new Error(selector);
  }
  return found;
}

describe("isFullScreenKey", () => {
  it("takes F and f with no modifier", () => {
    expect(isFullScreenKey(press({ key: "f", code: "KeyF" }))).toBe(true);
    expect(isFullScreenKey(press({ key: "F", code: "KeyF", shiftKey: true }))).toBe(true);
    expect(isFullScreenKey(press({ key: "f", code: "KeyF", ctrlKey: true }))).toBe(false);
    expect(isFullScreenKey(press({ key: "f", code: "KeyF", metaKey: true }))).toBe(false);
    expect(isFullScreenKey(press({ key: "f", code: "KeyF", repeat: true }))).toBe(false);
  });

  it("follows the letter in Latin layouts and the key's place in others", () => {
    // Dvorak types "u" where QWERTY has F, and F elsewhere.
    expect(isFullScreenKey(press({ key: "u", code: "KeyF" }))).toBe(false);
    expect(isFullScreenKey(press({ key: "f", code: "KeyY" }))).toBe(true);
    // Russian types "а" in F's place.
    expect(isFullScreenKey(press({ key: "а", code: "KeyF" }))).toBe(true);
  });

  it("leaves the key alone while the user types or works in a dialog", () => {
    for (const [html, selector] of [
      ["<input />", "input"],
      ["<textarea></textarea>", "textarea"],
      ['<div contenteditable="true"><span>x</span></div>', "span"],
      ['<div role="dialog"><button>OK</button></div>', "button"],
      ['<div role="menu"><div role="menuitem">A</div></div>', "[role=menuitem]"],
    ] as const) {
      expect(isFullScreenKey(press({ key: "f", code: "KeyF" }, inside(html, selector)))).toBe(
        false,
      );
    }
  });
});

describe("orientationFor", () => {
  it("turns to the picture's shape, landscape for a square, and nowhere before it has one", () => {
    expect(orientationFor(1920, 1080)).toBe("landscape");
    expect(orientationFor(1080, 2400)).toBe("portrait");
    expect(orientationFor(720, 720)).toBe("landscape");
    expect(orientationFor(0, 0)).toBeNull();
  });
});
