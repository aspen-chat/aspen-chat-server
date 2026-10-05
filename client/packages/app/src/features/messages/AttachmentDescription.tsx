import { ATTACHMENT_DESCRIPTION_MAX_CHARS } from "@aspen/protocol";
import { TextAlignLeftIcon } from "@phosphor-icons/react";
import { useState } from "react";
import {
  Button,
  Dialog,
  DialogTrigger,
  Form,
  Label,
  Modal,
  ModalOverlay,
  Text,
  TextArea,
  TextField,
} from "react-aria-components";
import { inputClass, labelClass, primaryButtonClass } from "@/features/auth/styles";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * The control on a file waiting to be sent that describes what it shows, for readers who cannot
 * see it: a button (filled once there is a description) opening a modal with the text, kept
 * when saved. What is saved, and when it reaches the server, is the message box's business.
 */
export function AttachmentDescriptionButton({
  name,
  description,
  onSave,
  className,
}: {
  name: string;
  description: string;
  onSave: (description: string) => void;
  className: string;
}) {
  const m = useMessages();
  const described = description.trim().length > 0;
  const label = format(described ? m.editAttachmentDescription : m.describeAttachment, { name });
  return (
    <DialogTrigger>
      <Tooltip text={label}>
        <Button aria-label={label} className={className}>
          <TextAlignLeftIcon
            size={14}
            weight={described ? "bold" : "regular"}
            aria-hidden="true"
            className={described ? "text-accent" : undefined}
          />
        </Button>
      </Tooltip>
      <ModalOverlay isDismissable className={overlayClass}>
        <Modal className={modalClass}>
          <Dialog className={dialogClass}>
            {({ close }) => (
              <DescriptionForm
                name={name}
                description={description}
                onSave={(text) => {
                  onSave(text);
                  close();
                }}
              />
            )}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

/** The modal's contents, made afresh each time it opens so it starts from what was saved. */
function DescriptionForm({
  name,
  description,
  onSave,
}: {
  name: string;
  description: string;
  onSave: (description: string) => void;
}) {
  const m = useMessages();
  const [text, setText] = useState(description);
  return (
    <Form
      onSubmit={(event) => {
        event.preventDefault();
        // React carries events up through the portal the modal is in, and the message box this
        // opens from is a form too, which would send the message.
        event.stopPropagation();
        onSave(text.trim());
      }}
      className="flex flex-col gap-4"
    >
      <DialogHeading>{format(m.describeAttachment, { name })}</DialogHeading>
      <TextField
        value={text}
        onChange={setText}
        maxLength={ATTACHMENT_DESCRIPTION_MAX_CHARS}
        autoFocus
        className="flex flex-col gap-1"
      >
        <Label className={labelClass}>{m.attachmentDescription}</Label>
        <TextArea rows={4} className={inputClass + " max-h-64 resize-none field-sizing-content"} />
        <Text slot="description" className="text-xs text-ink-muted">
          {m.attachmentDescriptionHint}
        </Text>
      </TextField>
      <div className="flex justify-end">
        <Button type="submit" className={primaryButtonClass}>
          {m.save}
        </Button>
      </div>
    </Form>
  );
}
