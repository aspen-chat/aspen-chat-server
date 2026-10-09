import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { PaneEdge } from "@/features/layout/ResizablePane";
import { HistorySkeleton } from "@/features/messages/MessageSkeleton";

/** Channel names' widths in a skeleton list, so its rows read as names rather than bars. */
const CHANNEL_WIDTHS = ["w-24", "w-32", "w-20", "w-28", "w-36", "w-24", "w-16", "w-28"];

/**
 * A community's channel list on its way, shaped like `ChannelSidebar`: the community's name,
 * then a category's heading and its channels.
 */
export function ChannelListSkeleton() {
  return (
    <section
      aria-busy="true"
      className="relative flex h-full flex-col border-e border-line bg-surface-raised"
    >
      <LoadingLabel />
      <div className="flex h-11 items-center border-b border-line px-4 py-2">
        <Skeleton className="h-4 w-32" />
      </div>
      <div className="flex flex-1 flex-col gap-2.5 overflow-hidden px-4 py-3">
        {CHANNEL_WIDTHS.map((width, index) => (
          <div key={index} className="flex items-center gap-2">
            {index === 3 ? (
              <Skeleton className="mt-2 h-3 w-16" />
            ) : (
              <>
                <Skeleton className="h-4 w-4 shrink-0 rounded" />
                <Skeleton className={"h-3.5 " + width} />
              </>
            )}
          </div>
        ))}
      </div>
      <PaneEdge />
    </section>
  );
}

/** A channel on its way, shaped like `ChannelScreen`: its header, then its history. */
export function ChannelSkeleton() {
  return (
    <main aria-busy="true" className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="flex h-[3.25rem] items-center gap-2 border-b border-line px-4 py-3">
        <Skeleton className="h-4 w-4 rounded" />
        <Skeleton className="h-4 w-36" />
      </div>
      <HistorySkeleton />
    </main>
  );
}

/**
 * A list of people or conversations on its way: rows of a picture and a name, `count` of them,
 * the picture `size` like the rows it stands for.
 */
export function RowsSkeleton({ count = 5, size = "sm" }: { count?: number; size?: "sm" | "md" }) {
  const picture = size === "md" ? "h-11 w-11" : "h-6 w-6";
  return (
    <div aria-busy="true" className="flex flex-col gap-0.5">
      <LoadingLabel />
      {Array.from({ length: count }, (_, index) => (
        <div key={index} className="flex items-center gap-2 px-2 py-1.5">
          <Skeleton className={picture + " shrink-0 rounded-full"} />
          <Skeleton
            className={"h-3.5 " + (CHANNEL_WIDTHS[index % CHANNEL_WIDTHS.length] ?? "w-24")}
          />
        </div>
      ))}
    </div>
  );
}
