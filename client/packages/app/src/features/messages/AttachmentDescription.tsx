import { ATTACHMENT_DESCRIPTION_MAX_CHARS } from "@aspen/protocol";
import { EyeSlashIcon, TextAlignLeftIcon } from "@phosphor-icons/react";
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
  ToggleButton,
} from "react-aria-components";
import { inputClass, labelClass, primaryButtonClass } from "@/features/auth/styles";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** The look of the small controls beside a file in the message box and the message editor. */
export const pendingControlClass =
  "rounded p-0.5 text-ink-faint outline-none hover:text-ink pointer-coarse:p-1.5 " +
  "focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * The control on a file, waiting to be sent or in a message being edited, that describes what it
 * shows, for readers who cannot see it: a button (filled once there is a description) opening a
 * modal with the text, kept when saved. What is saved, and when it reaches the server, is the
 * business of what holds it.
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

/**
 * Marks a file as a spoiler, which readers' apps cover until each reader
 * reveals it, or takes the mark off; its eye is filled while it is on.
 */
export function SpoilerToggle({
  name,
  spoiler,
  onChange,
}: {
  name: string;
  spoiler: boolean;
  onChange: (spoiler: boolean) => void;
}) {
  const m = useMessages();
  const label = format(m.markSpoiler, { name });
  return (
    <Tooltip text={label}>
      <ToggleButton
        aria-label={label}
        isSelected={spoiler}
        onChange={onChange}
        className={pendingControlClass + " selected:text-accent"}
      >
        <EyeSlashIcon size={14} weight={spoiler ? "fill" : "regular"} aria-hidden="true" />
      </ToggleButton>
    </Tooltip>
  );
}
