/**
 * An icon button in a sidebar's header, beside others like it. Its 18px icon and border make it
 * 44px on a touch screen with the larger padding, drawn big rather than given `tap-target`,
 * whose enlarged areas would overlap its neighbours'.
 */
export const headerIconButtonClass =
  "rounded-md border border-line p-1.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover disabled:opacity-40 focus-visible:ring-2 focus-visible:ring-accent/50 " +
  "pointer-coarse:p-3";
