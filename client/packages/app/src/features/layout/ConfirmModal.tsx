import { useState } from "react";
import { Button, Dialog, Modal, ModalOverlay } from "react-aria-components";
import { problemText } from "@/api/problemText";
import {
  dangerButtonClass,
  dialogClass,
  modalClass,
  overlayClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";

/**
 * Asks before something hard to take back (blocking someone, removing them, leaving a
 * community), saying what it does, then does it with `run`, the one button, in the danger
 * colour. It closes once `run` has done it, and says why when it could not.
 */
export function ConfirmModal({
  heading,
  explanation,
  action,
  run,
  isOpen,
  onOpenChange,
}: {
  heading: string;
  explanation: string;
  action: string;
  run: () => Promise<void>;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog role="alertdialog" className={dialogClass}>
          <DialogHeading>{heading}</DialogHeading>
          <p className="text-sm text-ink-muted">{explanation}</p>
          {error !== null && (
            <p role="alert" className="text-sm text-danger">
              {error}
            </p>
          )}
          <div className="flex justify-end">
            <Button
              isDisabled={busy}
              onPress={() => {
                setBusy(true);
                setError(null);
                run().then(
                  () => {
                    setBusy(false);
                    onOpenChange(false);
                  },
                  (failure: unknown) => {
                    setBusy(false);
                    setError(problemText(failure));
                  },
                );
              }}
              className={dangerButtonClass}
            >
              {action}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
