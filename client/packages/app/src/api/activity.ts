import type { AspenSync } from "@aspen/protocol";

/**
 * What counts as the user using the app: pointer, keyboard, wheel, and touch input, and the
 * window coming into view or gaining focus. Every one is passive and cheap to hear, since
 * `AspenSync.noteActivity` only compares timestamps until a report is due.
 */
const INPUT_EVENTS = ["pointerdown", "pointermove", "keydown", "wheel", "touchstart"] as const;

/**
 * Reports the user's activity to `sync` for as long as the returned function is not called,
 * so they show as online while they use the app and away once they stop. Opening the app in a
 * focused window counts as using it; opening it in the background does not.
 */
export function reportActivity(sync: AspenSync, target: Window = window): () => void {
  const note = () => {
    sync.noteActivity();
  };
  const onVisible = () => {
    if (target.document.visibilityState === "visible") {
      note();
    }
  };
  for (const type of INPUT_EVENTS) {
    target.addEventListener(type, note, { passive: true, capture: true });
  }
  target.addEventListener("focus", note);
  target.document.addEventListener("visibilitychange", onVisible);
  if (target.document.hasFocus()) {
    note();
  }
  return () => {
    for (const type of INPUT_EVENTS) {
      target.removeEventListener(type, note, { capture: true });
    }
    target.removeEventListener("focus", note);
    target.document.removeEventListener("visibilitychange", onVisible);
  };
}
