import { useGrowthKey } from "@/features/layout/motion";
import { mentionsText } from "@/features/mentions/mentions";
import { useMessages } from "@/i18n/context";

/**
 * The count of unread messages that tag the reader, beside a channel, a DM, or a community, or
 * of whatever else `title` names (the reports awaiting review). It is drawn only; the row it
 * marks says the count in its accessible name. Rendered with a count of 0 it draws nothing but
 * stays ready to pop when the first comes.
 */
export function MentionBadge({
  count,
  className = "",
  title,
}: {
  count: number;
  className?: string;
  title?: string;
}) {
  const m = useMessages();
  // It pops each time the count grows while it is drawn, a first tag included.
  const grown = useGrowthKey(count);
  if (count === 0) {
    return null;
  }
  return (
    <span
      key={grown}
      aria-hidden="true"
      title={title ?? mentionsText(m, count)}
      className={
        (grown > 0 ? "motion-pop " : "") +
        "flex h-[18px] min-w-[18px] shrink-0 items-center justify-center rounded-full bg-danger px-1 forced-colors:border text-[11px] leading-none font-semibold text-accent-contrast tabular-nums " +
        className
      }
    >
      {count > 99 ? "99+" : count}
    </span>
  );
}
