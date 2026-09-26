import { useState, type KeyboardEvent, type ReactNode } from "react";
import { useMessages } from "@/i18n/context";

/**
 * Inline text hidden behind a block until the reader clicks or presses it, after which it stays
 * shown. While hidden the content is out of the accessibility tree, so a screen reader hears
 * only that there is a spoiler.
 */
export function Spoiler({ children }: { children: ReactNode }) {
  const m = useMessages();
  const [revealed, setRevealed] = useState(false);
  if (revealed) {
    return <span className="spoiler spoiler-revealed">{children}</span>;
  }
  const reveal = () => {
    setRevealed(true);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLSpanElement>) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      reveal();
    }
  };
  return (
    <span
      role="button"
      tabIndex={0}
      aria-label={m.revealSpoiler}
      onClick={reveal}
      onKeyDown={onKeyDown}
      className="spoiler spoiler-hidden"
    >
      <span aria-hidden="true">{children}</span>
    </span>
  );
}
