import { Dialog, Modal, ModalOverlay } from "react-aria-components";
import { OtherServerForm } from "@/features/deployments/OtherServerForm";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import type { OtherServerPrefill } from "@/features/invites/joinRouting";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";

/**
 * The add-a-server form in a dialog of its own, for a place outside the add dialog that was
 * handed another deployment's registration invite: filled in with `prefill`, which its owner
 * keeps while the dialog closes, so the form is there until the dialog has gone.
 */
export function OtherServerDialog({
  prefill,
  isOpen,
  onOpenChange,
}: {
  prefill: OtherServerPrefill | null;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>{m.deployments.useOther}</DialogHeading>
          {prefill !== null && (
            <OtherServerForm
              onDone={() => {
                onOpenChange(false);
              }}
              initial={prefill}
            />
          )}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
