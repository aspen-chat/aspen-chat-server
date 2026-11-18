import type { AspenSync } from "@aspen/protocol";

/**
 * What counts as the user using the app: pointer, keyboard, wheel, and touch input, and the
 * window coming into view or gaining focus. Every one is passive and cheap to hear, since
 * `AspenSync.noteActivity` and `noteAttending` only compare timestamps until a report is due.
 */
const INPUT_EVENTS = ["pointerdown", "pointermove", "keydown", "wheel", "touchstart"] as const;

/**
 * Reports the user's activity to `sync` for as long as the returned function is not called,
 * so they show as online while they use the app and away once they stop. Opening the app in a
 * focused window counts as using it; opening it in the background does not.
 *
 * It also says whether the app is in front of them, shown and focused, which keeps their phones
 * from being woken while they use it: the mobile app going to the background, or the desktop's
 * window losing focus, says at once that it no longer is.
 */
export function reportActivity(sync: AspenSync, target: Window = window): () => void {
  const document = target.document;
  const attended = () => document.visibilityState === "visible" && document.hasFocus();
  const note = () => {
    sync.noteActivity();
    if (attended()) {
      sync.noteAttending();
    }
  };
  const onVisibility = () => {
    if (document.visibilityState === "visible") {
      note();
    } else {
      sync.noteNotAttending();
    }
  };
  // Focus moving into a frame of the page (a plugin's view) blurs the window yet leaves the
  // document focused, which the next task can tell.
  const onBlur = () => {
    setTimeout(() => {
      if (!attended()) {
        sync.noteNotAttending();
      }
    }, 0);
  };
  const onHide = () => {
    sync.noteNotAttending();
  };
  for (const type of INPUT_EVENTS) {
    target.addEventListener(type, note, { passive: true, capture: true });
  }
  target.addEventListener("focus", note);
  target.addEventListener("blur", onBlur);
  target.addEventListener("pagehide", onHide);
  document.addEventListener("visibilitychange", onVisibility);
  if (document.hasFocus()) {
    note();
  }
  return () => {
    for (const type of INPUT_EVENTS) {
      target.removeEventListener(type, note, { capture: true });
    }
    target.removeEventListener("focus", note);
    target.removeEventListener("blur", onBlur);
    target.removeEventListener("pagehide", onHide);
    document.removeEventListener("visibilitychange", onVisibility);
    sync.noteNotAttending();
  };
}
