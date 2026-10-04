import { Dialog, Modal, ModalOverlay } from "react-aria-components";
import { useSync, useVoiceCall } from "@/api/hooks";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";

/**
 * Shown when the user's call ended in a way they need telling about: the server ended it for
 * being idle (a day alone in it), a moderator removed them, or they may no longer be in it.
 * Lost and removed servers are rejoined on their own and need no dialog.
 */
export function VoiceEndedDialog() {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  const reason = call.endedReason;
  if (reason !== "idle" && reason !== "kicked" && reason !== "accessLost") {
    return null;
  }
  const [heading, hint] =
    reason === "kicked"
      ? [m.voice.kickedHeading, m.voice.kickedHint]
      : reason === "accessLost"
        ? [m.voice.accessLostHeading, m.voice.accessLostHint]
        : [m.voice.idleEndedHeading, m.voice.idleEndedHint];
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
          <DialogHeading>{heading}</DialogHeading>
          <p className="text-sm text-ink-muted">{hint}</p>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
