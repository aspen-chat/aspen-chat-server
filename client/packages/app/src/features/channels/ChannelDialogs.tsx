import type { Channel } from "@aspen/protocol";
import { useNavigate, useParams } from "@tanstack/react-router";
import { useState } from "react";
import {
  Button,
  Dialog,
  Form,
  Input,
  Label,
  Modal,
  ModalOverlay,
  NumberField,
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
import { useDomain, communityLink } from "@/features/messages/links";
import { problemText } from "@/api/problemText";
import { useDeploymentProfile } from "@/features/auth/deploymentProfile";

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

/**
 * Deletes a channel after saying what goes with it. Someone viewing the channel is taken to
 * its community.
 */
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
  const domain = useDomain();
  const { channelId: current } = useParams({ strict: false });
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
                  if (channel.community != null && current === channel.id) {
                    void navigate(communityLink(domain, channel.community));
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

/**
 * A community text channel's own mana (the server's `app::mana`), for those who may manage
 * channels: how much it earns back a second and how much it holds. Each left empty follows the
 * deployment's, which is also the most it may be.
 */
export function ChannelManaDialog({
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
  const deployment = useDeploymentProfile()?.mana;
  const [recharge, setRecharge] = useState(channel.manaRecharge ?? NaN);
  const [max, setMax] = useState(channel.manaMax ?? NaN);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const own = (value: number) => (Number.isNaN(value) ? null : value);
  const field = (
    value: number,
    onChange: (value: number) => void,
    label: string,
    ceiling: number | undefined,
  ) => (
    <NumberField
      value={value}
      onChange={onChange}
      minValue={1}
      {...(ceiling === undefined ? {} : { maxValue: ceiling })}
      formatOptions={{ maximumFractionDigits: 0 }}
      className={fieldClass}
    >
      <Label className={labelClass}>{label}</Label>
      <Input
        placeholder={
          ceiling === undefined ? "" : format(m.channelActions.manaDefault, { value: ceiling })
        }
        className={inputClass + " sm:max-w-40"}
      />
    </NumberField>
  );
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
            {format(m.channelActions.manaHeading, { channel: channel.name })}
          </DialogHeading>
          <p className={hintClass}>{m.channelActions.manaHint}</p>
          <Form
            onSubmit={(event) => {
              event.preventDefault();
              setSaving(true);
              setError(null);
              sync
                .setChannelMana(channel.id, { manaRecharge: own(recharge), manaMax: own(max) })
                .then(
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
            {field(recharge, setRecharge, m.channelActions.manaRecharge, deployment?.maxRecharge)}
            {field(max, setMax, m.channelActions.manaMax, deployment?.max)}
            {error !== null && (
              <p role="alert" className={alertClass}>
                {error}
              </p>
            )}
            <Button
              type="submit"
              isDisabled={saving}
              className={primaryButtonClass + " self-start"}
            >
              {m.channelActions.manaSave}
            </Button>
          </Form>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
