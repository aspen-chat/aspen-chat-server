/** Tailwind class strings shared by the modal dialogs. */
export const overlayClass =
  "overlay-inset fixed inset-0 z-10 flex items-center justify-center bg-black/40 entering:animate-in exiting:animate-out";
/**
 * The frame every modal is drawn in. It is never taller than the overlay leaves room for, and
 * scrolls within itself when its content is taller than that, as a long form is on a phone;
 * the scroll stays in the dialog rather than passing to the page behind it.
 */
const modalFrameClass =
  "max-h-full w-full overflow-y-auto overscroll-contain rounded-lg border border-line " +
  "p-4 shadow-xl outline-none sm:p-5";
export const modalClass = modalFrameClass + " bg-surface-raised max-w-md";
/** A modal with room for a list of results beside its controls, such as message search. */
export const listModalClass = modalFrameClass + " bg-surface-raised max-w-xl";
/** A modal with room for a grid of choices, such as the screen share picker. */
export const wideModalClass = modalFrameClass + " bg-surface-raised max-w-3xl";
/**
 * A modal of many sections, such as Settings, each drawn as a plane (`planeClass`) raised off
 * the modal's plainer ground, so the eye finds where one ends and the next begins. It is wide
 * enough for the planes to stand in two columns (`PlaneColumns`).
 */
export const planesModalClass = modalFrameClass + " bg-surface max-w-3xl";
/** A section drawn raised off its ground; it never splits across columns. */
export const planeClass =
  "flex flex-col gap-3 break-inside-avoid rounded-lg border border-line bg-surface-raised p-1.5 shadow-sm";
export const dialogClass = "flex flex-col gap-4 outline-none";
export const headingClass = "text-lg font-semibold";
export const dangerButtonClass =
  "rounded-md bg-danger px-3 py-1.5 text-sm font-medium text-accent-contrast outline-none " +
  "hover:opacity-90 pressed:opacity-80 disabled:opacity-60 focus-visible:ring-2 focus-visible:ring-danger/50";
export const secondaryButtonClass =
  "rounded-md border border-line px-3 py-1.5 text-sm outline-none hover:bg-surface-hover " +
  "pressed:bg-surface-hover disabled:opacity-60 focus-visible:ring-2 focus-visible:ring-accent/50";
/** A `Select`'s trigger, its list, and its options, as every select in the app draws them. */
export const selectButtonClass =
  "flex w-full items-center justify-between gap-2 rounded-md border border-line bg-surface px-3 py-2 text-start text-sm outline-none " +
  "focus-visible:ring-2 focus-visible:ring-accent/50 disabled:opacity-60";
export const optionClass =
  "cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover selected:font-medium selected:text-accent";
export const selectPopoverClass =
  "min-w-(--trigger-width) rounded-md border border-line bg-surface-raised p-1 shadow-lg";
