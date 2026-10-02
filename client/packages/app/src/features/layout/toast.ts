import { UNSTABLE_ToastQueue as ToastQueue } from "react-aria-components";

/** What a toast says. */
export interface Notice {
  text: string;
}

/** How long a toast stays; React Aria holds it while it is hovered or focused. */
const TOAST_MS = 3000;

/** The app's one queue of toasts, which `toast` adds to from anywhere and `Toasts` shows. */
export const toasts = new ToastQueue<Notice>({ maxVisibleToasts: 3 });

/**
 * Says `text` for a moment, in a toast at the top of the page: for what was done and is done
 * with (text copied), where nothing else on the page will say so.
 */
export function toast(text: string): void {
  toasts.add({ text }, { timeout: TOAST_MS });
}
