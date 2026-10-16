// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { FollowDocumentTitle, useDocumentTitle } from "./documentTitle";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

function Named({ page }: { page: string | null }) {
  useDocumentTitle(page);
  return null;
}

describe("useDocumentTitle", () => {
  it("titles the page after the screen shown, and the app alone without one", () => {
    const root = createRoot(document.createElement("div"));
    const render = (screen: React.ReactNode) => {
      act(() => {
        root.render(
          <>
            <FollowDocumentTitle />
            {screen}
          </>,
        );
      });
    };
    render(null);
    expect(document.title).toBe("Aspen");
    render(<Named page={null} />);
    expect(document.title).toBe("Aspen");
    render(<Named page="Gardeners" />);
    expect(document.title).toBe("Gardeners · Aspen");
    render(<Named key="other" page="Direct messages" />);
    expect(document.title).toBe("Direct messages · Aspen");
    render(null);
    expect(document.title).toBe("Aspen");
    act(() => {
      root.unmount();
    });
  });
});
