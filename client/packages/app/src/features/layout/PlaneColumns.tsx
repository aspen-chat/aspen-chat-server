import type { ReactNode } from "react";

/**
 * Planes (`planeClass`) flowing down two balanced columns where there is room for two readable
 * ones, and down one where there is not. Their container's width decides, not the window's, so
 * a modal on a narrow window, or a pane beside others, gets one column.
 */
export function PlaneColumns({ children }: { children: ReactNode }) {
  return (
    <div className="@container">
      <div className="columns-1 gap-4 *:mb-4 *:last:mb-0 @2xl:columns-2">{children}</div>
    </div>
  );
}
