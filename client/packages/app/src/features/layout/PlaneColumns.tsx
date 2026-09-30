import type { ReactNode } from "react";

/**
 * Planes (`planeClass`) flowing down two balanced columns where there is room for two readable
 * ones, and down one where there is not. Their container's width decides, not the window's, so
 * a modal on a narrow window, or a pane beside others, gets one column. A plane never breaks
 * across the two: columns balance by splitting their content wherever it falls, which would
 * carry a plane's last controls to the top of the other column.
 */
export function PlaneColumns({ children }: { children: ReactNode }) {
  return (
    <div className="@container">
      <div className="columns-1 gap-4 *:mb-4 *:break-inside-avoid *:last:mb-0 @2xl:columns-2">
        {children}
      </div>
    </div>
  );
}
