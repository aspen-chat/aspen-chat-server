import { Button, Dialog, Modal, ModalOverlay } from "react-aria-components";
import { choosePush, dismissPushPrompt } from "@/api/pushChoice";
import {
  accentButtonClass,
  dialogClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";

/**
 * Asks whether this phone should be woken for new messages before the system is asked to let
 * Aspen notify, which happens only on Accept. Decline turns push off on this device until
 * Settings turns it on; the X, Escape, or a tap beside it leaves the question for the next start.
 */
export function PushPrompt() {
  const m = useMessages();
  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(open) => {
        if (!open) {
          dismissPushPrompt();
        }
      }}
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>{m.notifications.pushPrompt}</DialogHeading>
          <p className="text-sm text-ink-muted">{m.notifications.pushPromptHint}</p>
          <div className="flex justify-end gap-2">
            <Button
              onPress={() => {
                choosePush("off");
              }}
              className={secondaryButtonClass}
            >
              {m.notifications.pushDecline}
            </Button>
            <Button
              onPress={() => {
                choosePush("on");
              }}
              className={accentButtonClass}
            >
              {m.notifications.pushAccept}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
