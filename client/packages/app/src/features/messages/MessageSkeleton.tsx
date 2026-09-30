import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";

/** Line widths a run of skeleton messages cycles through, so they read as messages, not bars. */
const LINES: readonly (readonly string[])[] = [
  ["w-3/4"],
  ["w-11/12", "w-2/5"],
  ["w-1/2"],
  ["w-5/6", "w-3/4", "w-1/3"],
  ["w-2/3"],
];

/**
 * A message on its way, shaped like `MessageItem`: its author's picture, their name, and a line
 * or three of text.
 */
export function MessageSkeleton({ index = 0 }: { index?: number }) {
  const lines = LINES[index % LINES.length] ?? LINES[0] ?? [];
  return (
    <div className="flex gap-3 px-2 py-1.5">
      <Skeleton className="h-9 w-9 shrink-0 rounded-full" />
      <div className="flex min-w-0 flex-1 flex-col gap-1.5 pt-0.5">
        <Skeleton className={index % 2 === 0 ? "h-3.5 w-28" : "h-3.5 w-20"} />
        {lines.map((width, line) => (
          <Skeleton key={line} className={"h-3.5 " + width} />
        ))}
      </div>
    </div>
  );
}

/** A channel's history on its way: messages to fill the view, the newest at the bottom. */
export function HistorySkeleton({ count = 8 }: { count?: number }) {
  return (
    <div
      aria-busy="true"
      className="flex min-h-0 flex-1 flex-col justify-end gap-1 overflow-hidden px-4 py-3"
    >
      <LoadingLabel />
      {Array.from({ length: count }, (_, index) => (
        <MessageSkeleton key={index} index={index} />
      ))}
    </div>
  );
}
