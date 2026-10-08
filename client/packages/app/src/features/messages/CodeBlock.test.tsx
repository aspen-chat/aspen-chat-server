import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import { CodeBlock, MAX_HIGHLIGHT_LENGTH } from "./CodeBlock";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function draw(code: string): HTMLElement {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => {
    root.render(<CodeBlock code={code} language="javascript" />);
  });
  return container;
}

/** Lets the highlighter's work, a few promises long, finish. */
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 50));
  });
}

describe("CodeBlock", () => {
  it("highlights a block up to the limit and leaves a longer one plain", async () => {
    const short = draw("const a = 1;");
    await vi.waitFor(
      () => {
        expect(short.querySelector("code.hljs")).not.toBeNull();
      },
      {
        timeout: 10_000,
      },
    );

    const long = draw("const a = 1;\n".repeat(Math.ceil(MAX_HIGHLIGHT_LENGTH / 13) + 1));
    await settle();
    expect(long.querySelector("code.hljs")).toBeNull();
    expect(long.querySelector("code")?.textContent).toContain("const a = 1;");
  });
});
