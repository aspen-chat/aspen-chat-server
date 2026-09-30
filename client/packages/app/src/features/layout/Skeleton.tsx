import { useMessages } from "@/i18n/context";

/**
 * A stand-in for something on its way from the network: a block in the surface's hover shade
 * that pulses gently, still where the reader asks for less motion, sized like what it stands
 * for so that nothing moves when that arrives. Where the size cannot be known it is a
 * conservative guess. It is hidden from assistive technology; the region waiting says so with
 * `LoadingLabel`, and `aria-busy` where it is one element.
 */
export function Skeleton({
  className = "",
  inline = false,
}: {
  className?: string;
  /** Sits in a line of text, a word's height, rather than on a line of its own. */
  inline?: boolean;
}) {
  return (
    <span
      aria-hidden="true"
      className={
        (inline ? "inline-block h-[0.9em] align-middle " : "block ") +
        // Two rounding classes on one element are decided by the stylesheet's order, not
        // theirs, so the usual rounding stands only where the caller gives none.
        (/(^|\s)rounded/.test(className) ? "" : "rounded-md ") +
        "animate-pulse bg-surface-hover motion-reduce:animate-none " +
        className
      }
    />
  );
}

/** What a screen reader hears where skeletons stand: "Loading…", or `text`. */
export function LoadingLabel({ text }: { text?: string }) {
  const m = useMessages();
  return <span className="sr-only">{text ?? m.loading}</span>;
}
