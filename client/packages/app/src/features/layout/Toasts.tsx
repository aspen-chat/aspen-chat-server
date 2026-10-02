import { XIcon } from "@phosphor-icons/react";
import {
  Button,
  UNSTABLE_Toast as Toast,
  UNSTABLE_ToastContent as ToastContent,
  UNSTABLE_ToastRegion as ToastRegion,
} from "react-aria-components";
import { toasts } from "@/features/layout/toast";
import { useMessages } from "@/i18n/context";

/**
 * Where toasts show: a region over the top of what holds it (the channel's messages), each
 * toast dropping into it. What holds it is positioned.
 */
export function Toasts() {
  const m = useMessages();
  return (
    <ToastRegion
      queue={toasts}
      aria-label={m.toastsLabel}
      className="pointer-events-none absolute inset-x-0 top-3 z-20 flex flex-col items-center gap-2 px-4"
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
