import type { ReactNode } from "react";

/**
 * The head of every list sidebar, the channel list's and the DM list's alike: its title, which
 * names the sidebar's landmark (`headingId`), and its controls, each `headerIconButtonClass`,
 * 16px apart so their 44px touch areas stay clear of each other. It is as tall as the bar over
 * the conversation beside it (`pane-head-height`), so their rules meet.
 */
export function SidebarHeader({
  headingId,
  title,
  children,
}: {
  headingId: string;
  title: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="pane-head-height flex items-center gap-2 border-b border-line py-2 ps-4 pe-6">
      <h1 id={headingId} className="min-w-0 flex-1 truncate font-semibold">
        {title}
      </h1>
      <div className="flex shrink-0 items-center gap-4">{children}</div>
    </div>
  );
}
