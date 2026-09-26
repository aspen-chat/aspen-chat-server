import { ApiProblemError } from "@aspen/protocol";
import { TrashIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button, Dialog, DialogTrigger, Heading, Modal, ModalOverlay } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import {
  dialogClass,
  headingClass,
  modalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";

const dangerButtonClass =
  "rounded-md bg-danger px-3 py-1.5 text-sm font-medium text-accent-contrast outline-none " +
  "hover:opacity-90 pressed:opacity-80 disabled:opacity-60 focus-visible:ring-2 focus-visible:ring-danger/50";

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
          <TrashIcon size={16} aria-hidden="true" />
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
      <Heading slot="title" className={headingClass}>
        {m.deleteMessageHeading}
      </Heading>
      <p className="text-sm text-ink-muted">{m.deleteMessageHint}</p>
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      <div className="flex justify-end gap-2">
        <Button onPress={close} className={secondaryButtonClass}>
          {m.cancel}
        </Button>
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
