import { OverlayArrow, Tooltip as AriaTooltip, TooltipTrigger } from "react-aria-components";
import type { ReactNode } from "react";

/**
 * A hover and focus tooltip for an icon-only control. Wrap the control: the child must be a
 * React Aria `Button` (or another focusable trigger) so the tooltip follows both the pointer
 * and keyboard focus. The text should match the control's accessible name.
 */
export function Tooltip({ text, children }: { text: string; children: ReactNode }) {
  return (
    <TooltipTrigger delay={400} closeDelay={0}>
      {children}
      <AriaTooltip
        offset={6}
        className="rounded-md border border-line bg-surface-raised px-2 py-1 text-xs text-ink shadow-md"
      >
        <OverlayArrow>
          <svg
            width={8}
            height={8}
            viewBox="0 0 8 8"
            aria-hidden="true"
            className="fill-surface-raised stroke-line"
          >
            <path d="M0 0 L4 4 L8 0" />
          </svg>
        </OverlayArrow>
        {text}
      </AriaTooltip>
    </TooltipTrigger>
  );
}
