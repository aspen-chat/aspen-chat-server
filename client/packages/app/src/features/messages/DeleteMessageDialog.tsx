import { ACTION_ICON } from "@/features/messages/actionIcon";
import { ApiProblemError } from "@aspen/protocol";
import { TrashIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button, Dialog, DialogTrigger, Modal, ModalOverlay } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import {
  dangerButtonClass,
  dialogClass,
  modalClass,
  overlayClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";

/** A message's Delete control and the confirmation it asks for. */
export function DeleteMessageDialog({
  messageId,
  triggerClassName,
}: {
  messageId: string;
  triggerClassName: string;
}) {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Tooltip text={m.deleteMessage}>
        <Button className={triggerClassName} aria-label={m.deleteMessage}>
          <TrashIcon size={ACTION_ICON} aria-hidden="true" />
        </Button>
      </Tooltip>
      <ModalOverlay className={overlayClass} isDismissable>
        <Modal className={modalClass}>
          <Dialog role="alertdialog" className={dialogClass}>
            {({ close }) => <Confirm messageId={messageId} close={close} />}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

function Confirm({ messageId, close }: { messageId: string; close: () => void }) {
  const m = useMessages();
  const sync = useSync();
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function confirm() {
    setPending(true);
    setError(null);
    try {
      await sync.deleteMessage(messageId);
      close();
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
      setPending(false);
    }
  }

  return (
    <>
      <DialogHeading>{m.deleteMessageHeading}</DialogHeading>
      <p className="text-sm text-ink-muted">{m.deleteMessageHint}</p>
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <Button
          isDisabled={pending}
          onPress={() => {
            void confirm();
          }}
          className={dangerButtonClass}
        >
          {pending ? m.deleting : m.delete}
        </Button>
      </div>
    </>
  );
}
