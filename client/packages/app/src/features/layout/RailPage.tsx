import type { ReactNode } from "react";
import { SidebarFooter } from "@/features/layout/SidebarFooter";
import { PaneEdge, ResizablePane } from "@/features/layout/ResizablePane";
import type { PaneSizing } from "@/features/layout/paneSizes";

/**
 * A page of its own beside the community rail, as the Administration Dashboard and the
 * activity and saved messages pages are: a rail of what the page holds (`rail`, in a `nav`
 * named `navLabel`), the page's content, and the user bar at the rail's foot. On a one-pane
 * screen the rail runs across the top, the content below it, and the user bar last. The content
 * scrolls up and down only: what is wider than it (a code block, a table) scrolls in itself.
 */
export function RailPage({
  sizing,
  paneLabel,
  navLabel,
  rail,
  contentClassName,
  children,
}: {
  sizing: PaneSizing;
  /** What the rail's resizable pane is called. */
  paneLabel: string;
  navLabel: string;
  rail: ReactNode;
  /** The content's own column: its width and spacing. */
  contentClassName: string;
  children: ReactNode;
}) {
  return (
    // A grid where the rail stands beside the content, so the user bar can sit at the rail's
    // foot while staying last in reading order, as it is on a one-pane screen.
    // The rail's column is as wide as the rail's pane, which the user bar below it fills
    // without widening.
    <main className="flex min-w-0 flex-1 flex-col bg-surface md:grid md:grid-cols-[auto_minmax(0,1fr)] md:grid-rows-[minmax(0,1fr)_auto]">
      <ResizablePane
        sizing={sizing}
        edge="end"
        label={paneLabel}
        className="flex shrink-0 flex-col md:col-start-1 md:row-start-1 md:min-h-0"
      >
        <nav
          aria-label={navLabel}
          className="relative flex flex-col gap-2 border-b border-line bg-surface-sunken p-3 md:min-h-0 md:flex-1 md:border-e md:border-b-0"
        >
          {rail}
          <PaneEdge />
        </nav>
      </ResizablePane>
      <div className="min-h-0 min-w-0 flex-1 overflow-x-clip overflow-y-auto md:col-start-2 md:row-span-2 md:row-start-1">
        <div className={contentClassName}>{children}</div>
      </div>
      <div className="shrink-0 bg-surface-sunken md:col-start-1 md:row-start-2 md:w-0 md:min-w-full md:border-e md:border-line">
        <SidebarFooter groundClassName="bg-surface-sunken" />
      </div>
    </main>
  );
}
