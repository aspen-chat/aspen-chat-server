import { EyeIcon } from "@phosphor-icons/react";
import { useState, type ReactNode } from "react";
import { Button } from "react-aria-components";
import { useMessages } from "@/i18n/context";

/**
 * A link's card for a link inside a spoiler, blurred past reading under a Reveal spoiler
 * button until the reader presses it, after which it stays shown. While hidden the card is
 * `inert`: its links and play control take no pointer or focus, and assistive technology
 * hears only the button. The card's own top margin is taken by this wrapper, so the cover
 * lies on the card alone.
 */
export function SpoilerCard({ children }: { children: ReactNode }) {
  const m = useMessages();
  const [revealed, setRevealed] = useState(false);
  if (revealed) {
    return children;
  }
  return (
    <div className="relative mt-1 w-full max-w-lg overflow-hidden rounded-md [&>[inert]>*]:mt-0">
      <div inert className="pointer-events-none blur-xl select-none">
        {children}
      </div>
      <Button
        onPress={() => {
          setRevealed(true);
        }}
        className="absolute inset-0 flex items-center justify-center bg-surface-sunken/40 outline-none focus-visible:ring-2 focus-visible:ring-accent/60 focus-visible:ring-inset"
      >
        <span className="flex items-center gap-1.5 rounded-full bg-black/70 px-3 py-1.5 text-sm font-medium text-white shadow">
          <EyeIcon size={16} weight="bold" aria-hidden="true" />
          {m.revealSpoilerCard}
        </span>
      </Button>
    </div>
  );
}
