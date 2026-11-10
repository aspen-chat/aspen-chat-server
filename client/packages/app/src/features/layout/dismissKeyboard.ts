import { useEffect, useRef } from "react";

/**
 * Whether `element` raises a phone's keyboard when it holds the focus: a text field, or text
 * that is edited in place.
 */
export function raisesKeyboard(element: Element | null): boolean {
  if (element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) {
    return true;
  }
  return element instanceof HTMLElement && element.isContentEditable;
}

/** Whether a phone's keyboard is up, or about to be: a field that raises one holds the focus. */
export function keyboardRaised(): boolean {
  return raisesKeyboard(document.activeElement);
}

const listeners = new Set<() => void>();

/**
 * Puts the on-screen keyboard away, as a drag on the message list does where the list scrolls
 * itself (`ListScroller`), as a drag on iOS's own lists does: the field holding the focus lets
 * it go, so the phone's keyboard goes with it, and whatever stands in a keyboard's place (the
 * emoji keyboard docked under a message box on a touch screen) is told to go too
 * (`useOnKeyboardDismissed`).
 */
export function dismissKeyboard(): void {
  const active = document.activeElement;
  if (active instanceof HTMLElement && raisesKeyboard(active)) {
    active.blur();
  }
  for (const listener of listeners) {
    listener();
  }
}

/** Runs the latest `listener` each time the keyboard is put away (`dismissKeyboard`). */
export function useOnKeyboardDismissed(listener: () => void): void {
  const latest = useRef(listener);
  useEffect(() => {
    latest.current = listener;
  });
  useEffect(() => {
    const run = () => {
      latest.current();
    };
    listeners.add(run);
    return () => {
      listeners.delete(run);
    };
  }, []);
}
