import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { Markdown } from "./Markdown";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function draw(content: string): HTMLElement {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => {
    root.render(<Markdown content={content} />);
  });
  return container;
}

describe("Spoiler", () => {
  it("keeps hidden content inert and out of sight, code included", () => {
    const body = draw("||`code` and https://hidden.example||");
    const content = body.querySelector(".spoiler-hidden > .spoiler-content");
    expect(content?.hasAttribute("inert")).toBe(true);
    expect(content?.querySelector("code")).not.toBeNull();
    expect(content?.querySelector("a")).not.toBeNull();
  });

  it("only reveals on a click that lands on a hidden link, never following it", () => {
    const body = draw("||https://hidden.example||");
    const link = body.querySelector("a");
    if (link === null) {
      throw new Error("no link");
    }
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    act(() => {
      link.dispatchEvent(click);
    });
    expect(click.defaultPrevented).toBe(true);
    expect(body.querySelector(".spoiler-revealed")).not.toBeNull();

    const shown = body.querySelector("a");
    const second = new MouseEvent("click", { bubbles: true, cancelable: true });
    shown?.dispatchEvent(second);
    expect(second.defaultPrevented).toBe(false);
  });
});
