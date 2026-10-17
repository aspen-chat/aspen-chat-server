import { useEffect, useSyncExternalStore } from "react";
import { Button } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { isOutdated, reloadForNewRelease } from "./releaseReload";

/**
 * A strip above the app once its deployment serves a newer release than this page's, which
 * offers to reload, and reloads by itself the next time the page is hidden, when no one is in
 * the middle of anything. A page of an older release keeps working until a part it loads later
 * has gone from the server; `vite:preloadError` (`main.tsx`) reloads then.
 */
export function NewRelease() {
  const m = useMessages();
  const sync = useSync();
  const served = useSyncExternalStore(sync.subscribe, () => sync.webClientBuild);
  const outdated = isOutdated(served);
  useEffect(() => {
    if (!outdated) {
      return;
    }
    const reloadWhenHidden = () => {
      if (document.visibilityState === "hidden") {
        reloadForNewRelease();
      }
    };
    document.addEventListener("visibilitychange", reloadWhenHidden);
    return () => {
      document.removeEventListener("visibilitychange", reloadWhenHidden);
    };
  }, [outdated]);
  if (!outdated) {
    return null;
  }
  return (
    <div
      role="status"
      className="motion-drop flex items-center justify-center gap-3 bg-accent-soft px-3 py-1 text-sm text-ink"
    >
      <span>{m.newRelease.ready}</span>
      <Button
        onPress={() => {
          reloadForNewRelease();
        }}
        className="rounded border border-current px-2 py-0.5 text-xs outline-none pressed:opacity-70 focus-visible:ring-2 focus-visible:ring-accent/50"
      >
        {m.newRelease.reload}
      </Button>
    </div>
  );
}
