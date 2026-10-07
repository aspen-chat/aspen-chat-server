// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ErrorBoundary } from "./ErrorBoundary";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function Fails({ fail }: { fail: boolean }) {
  if (fail) {
    throw new RangeError("Maximum call stack size exceeded");
  }
  return <p>drawn</p>;
}

describe("ErrorBoundary", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("shows its fallback when its children throw, and tries them again for a new key", () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    const container = document.createElement("div");
    const root = createRoot(container);
    act(() => {
      root.render(
        <div>
          <ErrorBoundary fallback={<p>plain</p>} resetKey={1}>
            <Fails fail />
          </ErrorBoundary>
          <p>sibling</p>
        </div>,
      );
    });
    expect(container.textContent).toBe("plainsibling");
    act(() => {
      root.render(
        <div>
          <ErrorBoundary fallback={<p>plain</p>} resetKey={2}>
            <Fails fail={false} />
          </ErrorBoundary>
          <p>sibling</p>
        </div>,
      );
    });
    expect(container.textContent).toBe("drawnsibling");
    act(() => {
      root.unmount();
    });
  });
});
