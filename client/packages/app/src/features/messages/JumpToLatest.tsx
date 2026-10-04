import { useState } from "react";
import { Button } from "react-aria-components";
import { useMessages } from "@/i18n/context";

/**
 * The pill that brings a channel back to its newest messages. It keeps its own state, so a press
 * repaints the pill alone, saying the newest are on their way, and `onJump`, which sets the
 * whole list moving, starts only once that is on screen.
 */
export function JumpToLatest({ onJump }: { onJump: () => Promise<void> }) {
  const m = useMessages();
  const [jumping, setJumping] = useState(false);
  return (
    <Button
      onPress={() => {
        setJumping(true);
        requestAnimationFrame(() => {
          setTimeout(() => {
            void onJump().finally(() => {
              setJumping(false);
            });
          }, 0);
        });
      }}
      isPending={jumping}
      className="motion-rise pointer-events-auto flex shrink-0 items-center gap-2 rounded-full bg-accent px-4 py-1.5 text-sm font-medium text-accent-contrast shadow outline-none hover:bg-accent-strong pressed:opacity-80 focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      {jumping && (
        <span
          aria-hidden="true"
          className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-accent-contrast/40 border-t-accent-contrast"
        />
      )}
      {jumping ? m.jumpingToLatest : m.jumpToLatest}
    </Button>
  );
}
