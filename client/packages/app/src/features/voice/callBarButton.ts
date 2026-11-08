/** The shape every call bar button shares: an equal share of its row, however many the call offers. */
const callBarShapeClass =
  "flex min-w-0 flex-1 justify-center rounded-md py-2 outline-none pointer-coarse:py-3";

/** A call bar button in the muted ink, grounded on hover. */
export const callBarButtonClass =
  callBarShapeClass +
  " text-ink-muted hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";

/** Leaving the call, filled in the danger colour as the call screen's Leave Call button is. */
export const callBarLeaveClass =
  callBarShapeClass +
  " bg-danger text-accent-contrast hover:opacity-90 pressed:opacity-80 " +
  "focus-visible:ring-2 focus-visible:ring-danger/50";
