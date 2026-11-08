import { lazy, Suspense, type ComponentProps } from "react";
import { useMessages } from "@/i18n/context";

/** The emoji picker is a sizeable chunk, fetched the first time anyone opens it. */
const EmojiPicker = lazy(() => import("@/features/messages/EmojiPicker"));

/** The width of a popover's picker while it loads, near the width it takes once it has. */
const POPOVER_LOADING_WIDTH_PX = 320;

/**
 * `EmojiPicker`, fetched as it is first shown, with a note holding its room until it has
 * come: `height` tall, across its box where it `fill`s, else the width of a popover's picker.
 */
export function LazyEmojiPicker(props: ComponentProps<typeof EmojiPicker> & { height: number }) {
  const m = useMessages();
  return (
    <Suspense
      fallback={
        <div
          className="flex items-center justify-center text-sm text-ink-muted"
          style={{ width: props.fill ? "100%" : POPOVER_LOADING_WIDTH_PX, height: props.height }}
        >
          {m.loading}
        </div>
      }
    >
      <EmojiPicker {...props} />
    </Suspense>
  );
}
