import { Dialog, Modal, ModalOverlay } from "react-aria-components";
import { useSync, useVoiceCall } from "@/api/hooks";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";

/**
 * Shown when the user's call ended in a way they need telling about: the server ended it for
 * being idle (a day alone in it), or a moderator removed them. Lost and removed servers are
 * rejoined on their own and need no dialog.
 */
export function VoiceEndedDialog() {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  if (call.endedReason !== "idle" && call.endedReason !== "kicked") {
    return null;
  }
  const kicked = call.endedReason === "kicked";
  const dismiss = () => {
    sync.voice.acknowledgeEnd();
  };
  return (
    <ModalOverlay
      isOpen
      onOpenChange={(open) => {
        if (!open) {
          dismiss();
        }
      }}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog role="alertdialog" className={dialogClass}>
          <DialogHeading>{kicked ? m.voice.kickedHeading : m.voice.idleEndedHeading}</DialogHeading>
          <p className="text-sm text-ink-muted">
            {kicked ? m.voice.kickedHint : m.voice.idleEndedHint}
          </p>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
