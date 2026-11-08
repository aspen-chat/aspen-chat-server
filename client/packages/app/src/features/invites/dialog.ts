/**
 * The layer every modal overlay is drawn on: the z-index React Aria gives each popover, so
 * overlays are stacked in the order they open (each is portaled to the end of the body), and a
 * modal opened from a popover, such as a ban from a profile card, is drawn over it.
 */
export const overlayLayerClass = "z-100000";
/** Tailwind class strings shared by the modal dialogs. */
const overlayFrameClass =
  "fixed inset-x-0 top-[calc(var(--title-bar-height)+var(--visible-top))] bottom-(--visible-bottom) flex items-center justify-center bg-black/40 motion-backdrop " +
  overlayLayerClass;
export const overlayClass = "overlay-inset " + overlayFrameClass;
/**
 * The overlay of a modal whose frame is `touchFullScreenClass`: on a touch-only screen it leaves
 * no margin around the frame, only the notch and home indicator insets.
 */
export const fullScreenOverlayClass =
  "overlay-inset touch-only:overlay-inset-edge " + overlayFrameClass;
/**
 * The frame every modal is drawn in. It is never taller than the overlay leaves room for, and
 * scrolls within itself when its content is taller than that, as a long form is on a phone;
 * the scroll stays in the dialog rather than passing to the page behind it. Its title row
 * (`DialogHeading`) stays at the top while it scrolls, drawn on the ground `--modal-ground`
 * names, which each frame sets to its background (`pinned-title` in `styles.css`).
 */
// Clipped sideways: the heading's close button reaches a few pixels into the end padding for
// its tap area, which a scroll container would count as overflow and give a scrollbar for.
const modalFrameClass =
  "modal-frame max-h-full w-full overflow-x-hidden overflow-y-auto overscroll-contain rounded-lg " +
  "border border-line bg-(--modal-ground) shadow-xl outline-none motion-dialog";
const raisedGroundClass = "[--modal-ground:var(--color-surface-raised)]";
const plainGroundClass = "[--modal-ground:var(--color-surface)]";
export const modalClass = modalFrameClass + " " + raisedGroundClass + " max-w-md";
/** A modal with room for a list of results beside its controls, such as message search. */
export const listModalClass = modalFrameClass + " " + raisedGroundClass + " max-w-xl";
/** A modal with room for a grid of choices, such as the screen share picker. */
export const wideModalClass = modalFrameClass + " " + raisedGroundClass + " max-w-3xl";
/**
 * A modal of several sections, such as Sign-in and security, each drawn as a plane
 * (`planeClass`) raised off the modal's plainer ground, so the eye finds where one ends and the
 * next begins.
 */
export const planesModalClass = modalFrameClass + " " + plainGroundClass + " max-w-md";
/** The same, wider, for a modal that lays its planes out side by side, as Community settings. */
export const widePlanesModalClass = modalFrameClass + " " + plainGroundClass + " max-w-3xl";
/** The same, wide enough for the planes to stand in three columns (`PlaneColumns`), as Settings. */
export const columnPlanesModalClass = modalFrameClass + " " + plainGroundClass + " max-w-6xl";
/**
 * Added to a frame class for a modal that fills a touch-only screen edge to edge, with
 * `fullScreenOverlayClass` on its overlay: one that holds enough to fill a phone's screen
 * anyway, such as Settings or a list of results, which a margin and rounded corners would only
 * take room from. On any other screen it is the frame it is added to.
 */
export const touchFullScreenClass =
  "touch-only:h-full touch-only:max-w-none touch-only:rounded-none touch-only:border-0";
const planeLookClass = "break-inside-avoid rounded-lg border bg-surface-raised shadow-sm";
const planeShapeClass = planeLookClass + " px-3 py-1.5";
/** A plane's look alone, for an element that lays out its own contents, such as a list. */
export const planeSurfaceClass = planeShapeClass + " border-line";
/** A section drawn raised off its ground, its contents in a column; it never splits across columns. */
export const planeClass = "flex flex-col gap-3 " + planeSurfaceClass;
/**
 * `planeClass` with twice its padding, for contents that reach its edges, such as a slider whose
 * thumb would otherwise crowd the border at either end (the role editor's hue).
 */
export const roomyPlaneClass = "flex flex-col gap-3 border-line px-6 py-3 " + planeLookClass;
/** A plane holding something that cannot be undone, such as deleting a community. */
export const dangerPlaneClass = "flex flex-col gap-2 " + planeShapeClass + " border-danger/40";
export const dialogClass = "flex flex-col gap-4 outline-none";
export const headingClass = "text-lg font-semibold";
/** A dialog's or card's button for what is most likely wanted, beside `secondaryButtonClass`. */
export const accentButtonClass =
  "rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-accent-contrast outline-none " +
  "hover:bg-accent-strong pressed:opacity-80 disabled:opacity-60 focus-visible:ring-2 focus-visible:ring-accent/50";
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
/** A chip that toggles on and off, alone or as one of a group of choices. */
export const toggleChipClass =
  "rounded-md border border-line px-3 py-1 text-sm text-ink-muted outline-none hover:bg-surface-hover " +
  "pressed:bg-surface-hover selected:border-accent selected:bg-accent-soft selected:text-accent-strong " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";
