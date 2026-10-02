import { XIcon } from "@phosphor-icons/react";
import {
  Button,
  UNSTABLE_Toast as Toast,
  UNSTABLE_ToastContent as ToastContent,
  UNSTABLE_ToastRegion as ToastRegion,
} from "react-aria-components";
import { useEffect, useSyncExternalStore } from "react";
import { toasts } from "@/features/layout/toast";
import { useMessages } from "@/i18n/context";

/** How many regions over a channel's messages are mounted, for the root's fallback to know. */
let mounted = 0;
const listeners = new Set<() => void>();
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
function setMounted(count: number) {
  mounted = count;
  for (const listener of listeners) {
    listener();
  }
}

/**
 * Where toasts show: a region over the top of what holds it (the channel's messages), each
 * toast dropping into it; what holds it is positioned. The root mounts one as `fallback`,
 * fixed to the top of the page, which shows only while no channel's region is mounted (the
 * channel list on a phone, settings opened from it), so a toast is shown once wherever it
 * is sent from.
 */
export function Toasts({ fallback = false }: { fallback?: boolean }) {
  const m = useMessages();
  const inner = useSyncExternalStore(subscribe, () => mounted > 0);
  useEffect(() => {
    if (fallback) {
      return undefined;
    }
    setMounted(mounted + 1);
    return () => {
      setMounted(mounted - 1);
    };
  }, [fallback]);
  if (fallback && inner) {
    return null;
  }
  return (
    <ToastRegion
      queue={toasts}
      aria-label={m.toastsLabel}
      className={
        "pointer-events-none z-20 flex flex-col items-center gap-2 px-4 " +
        (fallback
          ? "fixed inset-x-0 top-[max(0.75rem,env(safe-area-inset-top))] z-50"
          : "absolute inset-x-0 top-3")
      }
    >
      {({ toast: shown }) => (
        <Toast
          toast={shown}
          className="motion-drop pointer-events-auto flex max-w-full items-center gap-2 rounded-full border border-line bg-surface-raised py-1.5 ps-4 pe-1.5 text-sm text-ink shadow-lg outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <ToastContent>
            <p>{shown.content.text}</p>
          </ToastContent>
          <Button
            slot="close"
            aria-label={m.close}
            className="rounded-full p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <XIcon size={16} aria-hidden="true" />
          </Button>
        </Toast>
      )}
    </ToastRegion>
  );
}
