import { mentionsText } from "@/features/mentions/mentions";
import { useMessages } from "@/i18n/context";

/**
 * The count of unread messages that tag the reader, beside a channel, a DM, or a community.
 * It is drawn only; the row it marks says the count in its accessible name.
 */
export function MentionBadge({ count, className = "" }: { count: number; className?: string }) {
  const m = useMessages();
  if (count === 0) {
    return null;
  }
  return (
    <span
      aria-hidden="true"
      title={mentionsText(m, count)}
      className={
        "flex h-[18px] min-w-[18px] shrink-0 items-center justify-center rounded-full bg-danger px-1 text-[11px] leading-none font-semibold text-accent-contrast tabular-nums " +
        className
      }
    >
      {count > 99 ? "99+" : count}
    </span>
  );
}
