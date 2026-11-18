import type { AspenSync } from "@aspen/protocol";
import { afterEach, describe, expect, it, vi } from "vitest";
import { reportActivity } from "./activity";

function fakeSync() {
  const noteActivity = vi.fn();
  const noteAttending = vi.fn();
  const noteNotAttending = vi.fn();
  return {
    sync: { noteActivity, noteAttending, noteNotAttending } as unknown as AspenSync,
    noteActivity,
    noteAttending,
    noteNotAttending,
  };
}

function setVisibility(state: DocumentVisibilityState) {
  vi.spyOn(document, "visibilityState", "get").mockReturnValue(state);
  document.dispatchEvent(new Event("visibilitychange"));
}

afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

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
    const { sync, noteActivity, noteAttending } = fakeSync();
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const stop = reportActivity(sync);
    expect(noteActivity).toHaveBeenCalledTimes(1);
    expect(noteAttending).toHaveBeenCalledTimes(1);
    stop();
  });

  it("is attending only while the window is shown and focused", () => {
    const { sync, noteActivity, noteAttending } = fakeSync();
    const focused = vi.spyOn(document, "hasFocus").mockReturnValue(false);
    const stop = reportActivity(sync);
    window.dispatchEvent(new KeyboardEvent("keydown"));
    expect(noteActivity).toHaveBeenCalledTimes(1);
    expect(noteAttending).not.toHaveBeenCalled();
    focused.mockReturnValue(true);
    window.dispatchEvent(new KeyboardEvent("keydown"));
    expect(noteAttending).toHaveBeenCalledTimes(1);
    vi.spyOn(document, "visibilityState", "get").mockReturnValue("hidden");
    window.dispatchEvent(new KeyboardEvent("keydown"));
    expect(noteAttending).toHaveBeenCalledTimes(1);
    stop();
  });

  it("stops attending when the app goes to the background", () => {
    const { sync, noteNotAttending } = fakeSync();
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const stop = reportActivity(sync);
    setVisibility("hidden");
    expect(noteNotAttending).toHaveBeenCalledTimes(1);
    stop();
  });

  it("stops attending when the window loses focus, but not to a frame of its own", () => {
    vi.useFakeTimers();
    const { sync, noteNotAttending } = fakeSync();
    const focused = vi.spyOn(document, "hasFocus").mockReturnValue(true);
    const stop = reportActivity(sync);
    window.dispatchEvent(new FocusEvent("blur"));
    vi.runAllTimers();
    expect(noteNotAttending).not.toHaveBeenCalled();
    focused.mockReturnValue(false);
    window.dispatchEvent(new FocusEvent("blur"));
    vi.runAllTimers();
    expect(noteNotAttending).toHaveBeenCalledTimes(1);
    stop();
  });

  it("stops attending when stopped", () => {
    const { sync, noteNotAttending } = fakeSync();
    vi.spyOn(document, "hasFocus").mockReturnValue(true);
    reportActivity(sync)();
    expect(noteNotAttending).toHaveBeenCalledTimes(1);
  });
});
