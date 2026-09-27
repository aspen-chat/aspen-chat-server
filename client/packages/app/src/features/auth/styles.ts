/** Tailwind class strings shared by the authentication forms. */
export const fieldClass = "flex flex-col gap-1";
export const labelClass = "text-sm font-medium text-ink-muted";
export const inputClass =
  "rounded-md border border-line bg-surface-raised px-3 py-2 text-base outline-none " +
  "focus:border-accent focus:ring-2 focus:ring-accent/30 invalid:border-danger";
export const primaryButtonClass =
  "rounded-md bg-accent px-4 py-2 font-medium text-accent-contrast outline-none hover:bg-accent-strong " +
  "pressed:opacity-80 disabled:opacity-60 focus-visible:ring-2 focus-visible:ring-accent/50";
/** A full-width alternative beside a primary button, such as signing in with a passkey. */
export const outlineButtonClass =
  "flex items-center justify-center gap-2 rounded-md border border-line px-4 py-2 font-medium " +
  "outline-none hover:bg-surface-hover pressed:opacity-80 disabled:opacity-60 " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";
export const linkButtonClass =
  "font-medium text-accent underline-offset-2 outline-none hover:underline pressed:opacity-70 " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";
export const alertClass = "rounded-md bg-danger-soft px-3 py-2 text-sm text-danger";
export const hintClass = "text-sm text-ink-muted";
