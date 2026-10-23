import { useState, type KeyboardEvent, type MouseEvent, type ReactNode } from "react";
import { useMessages } from "@/i18n/context";

/**
 * Inline text hidden behind a block until the reader clicks or presses it, after which it stays
 * shown. While hidden the content is `inert`: a click on a link, tag, or chip in it lands on the
 * spoiler and only reveals it, nothing in it takes focus, and a screen reader hears only that
 * there is a spoiler. The reveal also cancels the click's default action, so a hidden link is
 * never followed on the click that reveals it.
 */
export function Spoiler({ children }: { children: ReactNode }) {
  const m = useMessages();
  const [revealed, setRevealed] = useState(false);
  if (revealed) {
    return <span className="spoiler spoiler-revealed">{children}</span>;
  }
  const reveal = (event: MouseEvent<HTMLSpanElement>) => {
    event.preventDefault();
    setRevealed(true);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLSpanElement>) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      setRevealed(true);
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
      <span inert className="spoiler-content">
        {children}
      </span>
    </span>
  );
}
