import { EyeIcon } from "@phosphor-icons/react";
import { useState, type ReactNode } from "react";
import { Button } from "react-aria-components";
import { useMessages } from "@/i18n/context";

/**
 * A link's card for a link inside a spoiler, covered (`SpoilerCover`) until the reader reveals
 * it, after which it stays shown. The card's own top margin is taken by this wrapper, so the
 * cover lies on the card alone.
 */
export function SpoilerCard({ children }: { children: ReactNode }) {
  const [revealed, setRevealed] = useState(false);
  if (revealed) {
    return children;
  }
  return (
    <SpoilerCover
      onReveal={() => {
        setRevealed(true);
      }}
      className="mt-1 w-full max-w-lg [&>[inert]>*]:mt-0"
    >
      {children}
    </SpoilerCover>
  );
}

/**
 * Something in a message blurred past reading under a Reveal spoiler button: a card, an
 * attachment. While covered it is `inert`: its links and controls take no pointer or focus, and
 * assistive technology hears only the button. It keeps its own room, so revealing it moves
 * nothing. `compact` draws the button's words without their pill, for a cover as small as a
 * line of text, which it is kept wide enough to hold.
 */
export function SpoilerCover({
  children,
  onReveal,
  compact = false,
  className = "",
}: {
  children: ReactNode;
  onReveal: () => void;
  compact?: boolean;
  className?: string;
}) {
  const m = useMessages();
  return (
    <div
      className={"relative overflow-hidden rounded-md " + (compact ? "min-w-36 " : "") + className}
    >
      <div inert className="pointer-events-none blur-xl select-none">
        {children}
      </div>
      <Button
        onPress={onReveal}
        className="absolute inset-0 flex items-center justify-center bg-surface-sunken/40 outline-none focus-visible:ring-2 focus-visible:ring-accent/60 focus-visible:ring-inset"
      >
        <span
          className={
            "flex items-center gap-1.5 text-sm font-medium " +
            (compact ? "text-ink" : "rounded-full bg-black/70 px-3 py-1.5 text-white shadow")
          }
        >
          <EyeIcon size={16} weight="bold" aria-hidden="true" />
          {m.revealSpoilerCard}
        </span>
      </Button>
    </div>
  );
}
