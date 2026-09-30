import { Button } from "react-aria-components";
import { useSync, useSyncStatus } from "@/api/hooks";
import { useMessages } from "@/i18n/context";

/**
 * A strip above the app while the cache is not current: bootstrapping, reconnecting, resyncing,
 * or failed. Invisible while live, which is nearly always.
 */
export function SyncBanner() {
  const m = useMessages();
  const sync = useSync();
  const status = useSyncStatus();
  if (status === "live" || status === "stopped") {
    return null;
  }
  const failed = status === "failed";
  return (
    <div
      role="status"
      className={
        "motion-drop flex items-center justify-center gap-3 px-3 py-1 text-sm " +
        (failed ? "bg-danger-soft text-danger" : "bg-accent-soft text-ink")
      }
    >
      <span>
        {failed ? (sync.lastError?.detail ?? sync.lastError?.title) : m.syncStatus[status]}
      </span>
      {failed && (
        <Button
          onPress={() => {
            sync.start();
          }}
          className="rounded border border-current px-2 py-0.5 text-xs outline-none pressed:opacity-70 focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          {m.retry}
        </Button>
      )}
    </div>
  );
}
