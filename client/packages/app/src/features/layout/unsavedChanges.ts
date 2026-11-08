import { createContext, useCallback, useContext, useId, useLayoutEffect } from "react";

/**
 * What the forms inside an `UnsavedChangesGuard` report to: whether each holds changes not yet
 * saved, by a key of its own, so several forms in one modal (the planes of Community settings)
 * each count. It is kept out of React state on purpose: whether a form is dirty changes with
 * every keystroke, and nothing is drawn differently for it until a close is requested.
 */
export interface UnsavedChangesRegistry {
  set(key: string, dirty: boolean): void;
  /** Whether any form has reported unsaved changes. */
  any(): boolean;
  clear(): void;
}

export function unsavedChangesRegistry(): UnsavedChangesRegistry {
  const dirtyKeys = new Set<string>();
  return {
    set(key, dirty) {
      if (dirty) {
        dirtyKeys.add(key);
      } else {
        dirtyKeys.delete(key);
      }
    },
    any: () => dirtyKeys.size > 0,
    clear: () => {
      dirtyKeys.clear();
    },
  };
}

export const UnsavedChangesContext = createContext<UnsavedChangesRegistry | null>(null);

/**
 * Tells the `UnsavedChangesGuard` around this form whether it holds changes not yet saved, so
 * the modal asks before closing on them; outside a guard it does nothing. The function it
 * returns says the changes are saved now, for the moment after a successful save when the
 * form's state still differs from the record and the modal is about to close: the guard's
 * record is updated at once, not after the next render.
 */
export function useUnsavedChanges(dirty: boolean): () => void {
  const registry = useContext(UnsavedChangesContext);
  const key = useId();
  useLayoutEffect(() => {
    registry?.set(key, dirty);
    return () => {
      registry?.set(key, false);
    };
  }, [registry, key, dirty]);
  return useCallback(() => {
    registry?.set(key, false);
  }, [registry, key]);
}
