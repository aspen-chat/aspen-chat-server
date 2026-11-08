import { useCallback, useState, type ReactNode } from "react";
import { ConfirmModal } from "@/features/layout/ConfirmModal";
import { UnsavedChangesContext, unsavedChangesRegistry } from "@/features/layout/unsavedChanges";
import { useMessages } from "@/i18n/context";

/** What a guarded modal is given to open and close by. */
export interface GuardedOpenState {
  isOpen: boolean;
  /**
   * The modal's `onOpenChange`: a request to close while something inside has unsaved changes
   * asks first, and closes only if the reader chooses to discard them.
   */
  onOpenChange: (open: boolean) => void;
  /** Closes without asking, for right after a save, when the form's state has not caught up. */
  close: () => void;
}

/**
 * Keeps a modal with a Save button from losing what was typed into it: when it is asked to
 * close (its X, Escape, a click beside it, or its owner) while a form inside holds changes that
 * were not saved, a confirmation asks whether to discard them, and the modal stays until the
 * reader says so. It owns the modal's open state, or follows `isOpen` and `onOpenChange` when
 * its owner does, and hands the modal what to open and close by through `children`:
 *
 *     <UnsavedChangesGuard>
 *       {(guard) => (
 *         <DialogTrigger isOpen={guard.isOpen} onOpenChange={guard.onOpenChange}>…</DialogTrigger>
 *       )}
 *     </UnsavedChangesGuard>
 *
 * A modal that is one form says whether it is dirty through `dirty`; the forms of a modal of
 * several report through `useUnsavedChanges`. Either way, closing after a successful save goes
 * through `guard.close()` or the hook's returned function, since the form still differs from
 * the record until the server's event reaches it, and the guard would otherwise ask about
 * changes that are already saved.
 */
export function UnsavedChangesGuard({
  dirty = false,
  isOpen,
  onOpenChange,
  children,
}: {
  dirty?: boolean;
  isOpen?: boolean;
  onOpenChange?: (open: boolean) => void;
  children: (guard: GuardedOpenState) => ReactNode;
}) {
  const m = useMessages();
  const [ownOpen, setOwnOpen] = useState(false);
  const open = isOpen ?? ownOpen;
  const [asking, setAsking] = useState(false);
  // One registry for the guard's life; the state never changes, only what it holds.
  const [registry] = useState(unsavedChangesRegistry);
  const change = useCallback(
    (next: boolean) => {
      if (!next) {
        registry.clear();
      }
      setOwnOpen(next);
      onOpenChange?.(next);
    },
    [registry, onOpenChange],
  );
  const request = useCallback(
    (next: boolean) => {
      if (!next && (dirty || registry.any())) {
        setAsking(true);
        return;
      }
      change(next);
    },
    [change, dirty, registry],
  );
  const close = useCallback(() => {
    change(false);
  }, [change]);
  return (
    <UnsavedChangesContext.Provider value={registry}>
      {children({ isOpen: open, onOpenChange: request, close })}
      <ConfirmModal
        heading={m.unsaved.heading}
        explanation={m.unsaved.explanation}
        action={m.unsaved.discard}
        isOpen={asking}
        onOpenChange={setAsking}
        run={() => {
          close();
          return Promise.resolve();
        }}
      />
    </UnsavedChangesContext.Provider>
  );
}
