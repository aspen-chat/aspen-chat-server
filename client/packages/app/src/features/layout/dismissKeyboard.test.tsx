import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { dismissKeyboard, keyboardRaised, useOnKeyboardDismissed } from "./dismissKeyboard";
(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("dismissKeyboard", () => {
  let mount: HTMLDivElement;
  let root: Root | null = null;

  beforeAll(() => {
    // jsdom leaves `isContentEditable` undefined; the attribute alone stands in for it here.
    Object.defineProperty(HTMLElement.prototype, "isContentEditable", {
      configurable: true,
      get(this: HTMLElement) {
        return this.getAttribute("contenteditable") === "true";
      },
    });
  });

  beforeEach(() => {
    mount = document.createElement("div");
    document.body.append(mount);
  });

  afterEach(() => {
    act(() => {
      root?.unmount();
    });
    root = null;
    mount.remove();
  });

  it("takes the focus from a field that raises a keyboard, and tells whoever listens", () => {
    const field = document.createElement("textarea");
    mount.append(field);
    field.focus();
    expect(keyboardRaised()).toBe(true);
    const told = vi.fn();
    function Listens() {
      useOnKeyboardDismissed(told);
      return null;
    }
    root = createRoot(mount);
    act(() => {
      root?.render(<Listens />);
    });

    dismissKeyboard();

    expect(document.activeElement).toBe(document.body);
    expect(keyboardRaised()).toBe(false);
    expect(told).toHaveBeenCalledTimes(1);
  });

  it("takes the focus from text edited in place", () => {
    const editable = document.createElement("div");
    editable.setAttribute("contenteditable", "true");
    editable.tabIndex = 0;
    mount.append(editable);
    editable.focus();
    expect(keyboardRaised()).toBe(true);

    dismissKeyboard();

    expect(document.activeElement).toBe(document.body);
  });

  it("leaves the focus on what raises no keyboard", () => {
    const button = document.createElement("button");
    mount.append(button);
    button.focus();
    expect(keyboardRaised()).toBe(false);

    dismissKeyboard();

    expect(document.activeElement).toBe(button);
  });

  it("stops telling a listener that has gone", () => {
    const told = vi.fn();
    function Listens() {
      useOnKeyboardDismissed(told);
      return null;
    }
    root = createRoot(mount);
    act(() => {
      root?.render(<Listens />);
    });
    act(() => {
      root?.unmount();
    });
    root = null;

    dismissKeyboard();

    expect(told).not.toHaveBeenCalled();
  });
});
