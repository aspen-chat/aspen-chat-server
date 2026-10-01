import { useEffect, useRef, useSyncExternalStore, type RefObject } from "react";
import { Button } from "react-aria-components";
import { copyText } from "@/features/layout/clipboard";
import type { ScrollDiagnostics } from "@/features/messages/scrollDiagnostics";

/**
 * The scroll diagnostics over the list, in a build made with `VITE_SCROLL_DEBUG=1`: how many
 * jumps its watcher has seen in the `viewport`, named `scroll-diagnostics` so a device test can
 * read it, the entries around the first jump out of sight for the same test, and a button that
 * copies the whole record for pasting into a report. Not localized, since it is a developer's
 * tool and never in a build people use.
 */
export function ScrollDiagnosticsPanel({
  diagnostics,
  viewport,
}: {
  diagnostics: ScrollDiagnostics;
  viewport: RefObject<HTMLElement | null>;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const jumps = useSyncExternalStore(
    (listener) => diagnostics.subscribe(listener),
    () => diagnostics.jumps,
  );
  const firstJump = useSyncExternalStore(
    (listener) => diagnostics.subscribe(listener),
    () => diagnostics.firstJump,
  );
  useEffect(() => {
    const box = viewport.current;
    if (box === null) {
      return;
    }
    return diagnostics.watch(box);
  }, [diagnostics, viewport]);
  return (
    <div
      ref={panel}
      className="pointer-events-auto absolute end-2 top-2 z-10 flex items-center gap-2 rounded bg-surface px-2 py-1 font-mono text-xs shadow"
    >
      <span>scroll-diagnostics jumps {String(jumps)}</span>
      {firstJump !== null && (
        // Out of sight, for a device test to read; the copied log holds it too.
        <span className="sr-only">scroll-jump-log {firstJump}</span>
      )}
      <Button
        className="rounded border border-line px-1"
        onPress={() => {
          if (panel.current !== null) {
            void copyText(diagnostics.text, panel.current);
          }
        }}
      >
        Copy log
      </Button>
    </div>
  );
}
