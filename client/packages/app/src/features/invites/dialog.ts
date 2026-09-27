/** Tailwind class strings shared by the modal dialogs. */
export const overlayClass =
  "fixed inset-0 z-10 flex items-center justify-center bg-black/40 p-4 entering:animate-in exiting:animate-out";
export const modalClass =
  "w-full max-w-md rounded-lg border border-line bg-surface-raised p-5 shadow-xl outline-none";
export const dialogClass = "flex flex-col gap-4 outline-none";
export const headingClass = "text-lg font-semibold";
export const secondaryButtonClass =
  "rounded-md border border-line px-3 py-1.5 text-sm outline-none hover:bg-surface-hover " +
  "pressed:bg-surface-hover disabled:opacity-60 focus-visible:ring-2 focus-visible:ring-accent/50";
/** A `Select`'s trigger and its options, as the settings and share dialogs draw them. */
export const selectButtonClass =
  "flex w-full items-center justify-between gap-2 rounded-md border border-line bg-surface px-3 py-2 text-left text-sm outline-none " +
  "focus-visible:ring-2 focus-visible:ring-accent/50 disabled:opacity-60";
export const optionClass =
  "cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover selected:font-medium selected:text-accent";
