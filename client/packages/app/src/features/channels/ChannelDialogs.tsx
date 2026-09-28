import { ApiProblemError, type Channel } from "@aspen/protocol";
import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import {
  Button,
  Dialog,
  Form,
  Input,
  Label,
  Modal,
  ModalOverlay,
  TextField,
} from "react-aria-components";
import { useSync } from "@/api/hooks";
import {
  alertClass,
  fieldClass,
  hintClass,
  inputClass,
  labelClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  dangerButtonClass,
  dialogClass,
  modalClass,
  overlayClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

function problemText(e: unknown): string {
  return e instanceof ApiProblemError ? e.message : String(e);
}

/** Renames a channel, for those who may manage channels or moderate the server. */
export function RenameChannelDialog({
  channel,
  isOpen,
  onOpenChange,
}: {
  channel: Channel;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [name, setName] = useState(channel.name);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>
            {format(m.channelActions.renameHeading, { channel: channel.name })}
          </DialogHeading>
          <Form
            onSubmit={(event) => {
              event.preventDefault();
              setSaving(true);
              setError(null);
              sync.renameChannel(channel.id, name.trim()).then(
                () => {
                  setSaving(false);
                  onOpenChange(false);
                },
                (e: unknown) => {
                  setSaving(false);
                  setError(problemText(e));
                },
              );
            }}
            className="flex flex-col gap-3"
          >
            <TextField value={name} onChange={setName} isRequired autoFocus className={fieldClass}>
              <Label className={labelClass}>{m.channelActions.nameLabel}</Label>
              <Input className={inputClass} />
            </TextField>
            {error !== null && (
              <p role="alert" className={alertClass}>
                {error}
              </p>
            )}
            <Button
              type="submit"
              isDisabled={saving || name.trim() === "" || name.trim() === channel.name}
              className={primaryButtonClass + " self-start"}
            >
              {m.channelActions.save}
            </Button>
          </Form>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/** Deletes a channel after saying what goes with it. */
export function DeleteChannelDialog({
  channel,
  isOpen,
  onOpenChange,
}: {
  channel: Channel;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const [deleting, setDeleting] = useState(false);
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
          <DialogHeading>
            {format(m.channelActions.deleteHeading, { channel: channel.name })}
          </DialogHeading>
          <p className={hintClass}>{m.channelActions.deleteHint}</p>
          {error !== null && (
            <p role="alert" className={alertClass}>
              {error}
            </p>
          )}
          <Button
            isDisabled={deleting}
            onPress={() => {
              setDeleting(true);
              setError(null);
              sync.deleteChannel(channel.id).then(
                () => {
                  onOpenChange(false);
                  if (channel.community != null) {
                    void navigate({
                      to: "/communities/$communityId",
                      params: { communityId: channel.community },
                    });
                  }
                },
                (e: unknown) => {
                  setDeleting(false);
                  setError(problemText(e));
                },
              );
            }}
            className={dangerButtonClass + " self-start"}
          >
            {m.channelActions.confirmDelete}
          </Button>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
