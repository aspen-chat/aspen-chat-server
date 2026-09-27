import type { AspenSync } from "@aspen/protocol";
import { describe, expect, it, vi } from "vitest";
import { reportActivity } from "./activity";

function fakeSync() {
  const noteActivity = vi.fn();
  return { sync: { noteActivity } as unknown as AspenSync, noteActivity };
}

describe("reportActivity", () => {
  it("notes input until stopped", () => {
    const { sync, noteActivity } = fakeSync();
    vi.spyOn(document, "hasFocus").mockReturnValue(false);
    const stop = reportActivity(sync);
    expect(noteActivity).not.toHaveBeenCalled();
    window.dispatchEvent(new KeyboardEvent("keydown"));
    window.dispatchEvent(new PointerEvent("pointermove"));
    expect(noteActivity).toHaveBeenCalledTimes(2);
    stop();
    window.dispatchEvent(new KeyboardEvent("keydown"));
    expect(noteActivity).toHaveBeenCalledTimes(2);
  });

  it("counts opening the app in a focused window", () => {
    const { sync, noteActivity } = fakeSync();
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const stop = reportActivity(sync);
    expect(noteActivity).toHaveBeenCalledTimes(1);
    stop();
  });
});
