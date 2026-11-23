import { useEffect, useState } from "react";

/**
 * Where the desktop app's updating stands (`packages/desktop/src/main/updates.ts`), followed as
 * it changes; `null` in a browser, on a phone, and in a desktop build that never updates itself.
 */
export function useDesktopUpdates(): DesktopUpdateState | null {
  const [state, setState] = useState<DesktopUpdateState | null>(null);
  useEffect(() => {
    const updates = window.aspenDesktop?.updates;
    if (updates === undefined) {
      return;
    }
    let live = true;
    const stop = updates.onChange((next) => {
      if (live) {
        setState(next);
      }
    });
    updates
      .state()
      .then((first) => {
        // A change heard while the first answer was on its way is the newer.
        if (live) {
          setState((heard) => heard ?? first);
        }
      })
      .catch((error: unknown) => {
        console.error("the update state could not be read", error);
      });
    return () => {
      live = false;
      stop();
    };
  }, []);
  return state;
}
