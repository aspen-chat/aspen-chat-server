/**
 * Puts `text` on the clipboard, and says whether it worked.
 *
 * The Clipboard API exists only on a secure page (HTTPS, or localhost), so on a page served over
 * plain HTTP, such as a dev server a phone reaches by its LAN address, it is missing. There the
 * text is copied the older way instead: selected in a hidden field and copied with the copy
 * command, which iOS Safari also honours. The field goes beside `near`, the control that asked,
 * because an open dialog keeps focus inside itself and would pull it back from a field placed
 * anywhere else before the copy ran.
 */
export async function copyText(text: string, near: Element): Promise<boolean> {
  if (window.isSecureContext && "clipboard" in navigator) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Refused (no permission, or no user gesture the browser accepts); try the older way.
    }
  }
  return copyBySelection(text, near);
}

function copyBySelection(text: string, near: Element): boolean {
  const field = document.createElement("textarea");
  field.value = text;
  // Read-only keeps the on-screen keyboard from opening; 16px keeps iOS from zooming in on it.
  field.readOnly = true;
  field.setAttribute("aria-hidden", "true");
  Object.assign(field.style, {
    position: "fixed",
    top: "0",
    left: "0",
    width: "1px",
    height: "1px",
    opacity: "0",
    fontSize: "16px",
  });
  const previous = document.activeElement;
  (near.parentElement ?? document.body).append(field);
  field.focus();
  field.select();
  // iOS selects nothing with `select()` alone.
  field.setSelectionRange(0, text.length);
  let copied: boolean;
  try {
    // Deprecated, but the one way to copy on a page without the Clipboard API.
    // eslint-disable-next-line @typescript-eslint/no-deprecated
    copied = document.execCommand("copy");
  } catch {
    copied = false;
  }
  field.remove();
  if (previous instanceof HTMLElement) {
    previous.focus();
  }
  return copied;
}
