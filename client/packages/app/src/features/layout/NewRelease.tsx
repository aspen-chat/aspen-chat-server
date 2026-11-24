import { useEffect, useSyncExternalStore } from "react";
import { useSync } from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { isOutdated, reloadForNewRelease } from "./releaseReload";
import { ReleaseStrip } from "./ReleaseStrip";

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
    <ReleaseStrip
      text={m.newRelease.ready}
      action={m.newRelease.reload}
      onAction={() => {
        reloadForNewRelease();
      }}
    />
  );
}
