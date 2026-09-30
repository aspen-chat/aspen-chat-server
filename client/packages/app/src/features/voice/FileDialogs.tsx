import type { OfferState, TransferMode } from "@aspen/protocol";
import { CaretDownIcon, FileIcon } from "@phosphor-icons/react";
import { type ReactNode, useState } from "react";
import {
  Button,
  Dialog,
  FileTrigger,
  Label,
  ListBox,
  ListBoxItem,
  Modal,
  ModalOverlay,
  Popover,
  RadioButton,
  RadioField,
  RadioGroup,
  Select,
  SelectValue,
} from "react-aria-components";
import { useLocale } from "react-aria-components";
import { useSync, useUser } from "@/api/hooks";
import {
  fieldClass,
  labelClass,
  outlineButtonClass,
  primaryButtonClass,
} from "@/features/auth/styles";
import {
  dialogClass,
  modalClass,
  optionClass,
  overlayClass,
  secondaryButtonClass,
  selectButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { choiceClass, RadioMark } from "@/features/layout/choices";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { handleOf } from "@/features/users/profile";
import {
  canChooseDestination,
  chooseDestination,
  formatSize,
  receiveModes,
  VALIDITY_CHOICES,
  type Validity,
} from "@/features/voice/files";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** One way a file can travel, as a radio choice with what it means beneath it. */
function Choice({
  value,
  label,
  hint,
  disabled = false,
}: {
  value: string;
  label: string;
  hint: ReactNode;
  disabled?: boolean;
}) {
  return (
    <RadioField value={value} isDisabled={disabled}>
      <RadioButton className={choiceClass}>
        <RadioMark />
        <span className="flex flex-col">
          <span className="font-medium">{label}</span>
          <span className="text-xs text-ink-muted">{hint}</span>
        </span>
      </RadioButton>
    </RadioField>
  );
}

function useLimit(relayMbps: number | null): string {
  const m = useMessages();
  const { locale } = useLocale();
  return relayMbps === null
    ? ""
    : format(m.files.mbps, { mbps: new Intl.NumberFormat(locale).format(relayMbps) });
}

/**
 * Offering a file to the call: the file, how long the offer stands, and how people who accept
 * may receive it, which the sender must choose: directly (which could expose their address) or
 * through the relay only. The relay choice is unavailable, and says so, on a server that does
 * not relay.
 */
export function OfferFileDialog({
  onClose,
  relayMbps,
}: {
  onClose: () => void;
  relayMbps: number | null;
}) {
  const m = useMessages();
  const sync = useSync();
  const { locale } = useLocale();
  const limit = useLimit(relayMbps);
  const [file, setFile] = useState<File | null>(null);
  const [validity, setValidity] = useState<Validity>(VALIDITY_CHOICES[0]);
  const [how, setHow] = useState<"direct" | "relay" | null>(null);
  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>{m.files.offerHeading}</DialogHeading>
          <p className="text-sm text-ink-muted">{m.files.offerHint}</p>
          <div className="flex items-center gap-3">
            <FileTrigger
              onSelect={(files) => {
                const chosen = files?.[0];
                if (chosen !== undefined) {
                  setFile(chosen);
                }
              }}
            >
              <Button className={outlineButtonClass}>
                {file === null ? m.files.chooseFile : m.files.changeFile}
              </Button>
            </FileTrigger>
            <span className="flex min-w-0 items-center gap-1 text-sm">
              {file === null ? (
                <span className="text-ink-muted">{m.files.noFile}</span>
              ) : (
                <>
                  <FileIcon size={16} aria-hidden="true" className="shrink-0" />
                  <span className="truncate">{file.name}</span>
                  <span className="shrink-0 text-ink-muted">{formatSize(file.size, locale)}</span>
                </>
              )}
            </span>
          </div>
          <Select
            value={validity}
            onChange={(key) => {
              if (
                typeof key === "string" &&
                (VALIDITY_CHOICES as readonly string[]).includes(key)
              ) {
                setValidity(key as Validity);
              }
            }}
            className={fieldClass}
          >
            <Label className={labelClass}>{m.files.validFor}</Label>
            <Button className={selectButtonClass}>
              <SelectValue />
              <CaretDownIcon size={14} aria-hidden="true" />
            </Button>
            <Popover className={selectPopoverClass}>
              <ListBox className="outline-none">
                {VALIDITY_CHOICES.map((choice) => (
                  <ListBoxItem
                    key={choice}
                    id={choice}
                    textValue={m.files.validity[choice]}
                    className={optionClass}
                  >
                    {m.files.validity[choice]}
                  </ListBoxItem>
                ))}
              </ListBox>
            </Popover>
          </Select>
          <RadioGroup
            value={how}
            onChange={(value) => {
              setHow(value === "direct" ? "direct" : "relay");
            }}
            className="flex flex-col gap-2"
          >
            <Label className={labelClass}>{m.files.senderChoice}</Label>
            <Choice value="direct" label={m.files.allowDirect} hint={m.files.allowDirectHint} />
            <Choice
              value="relay"
              label={m.files.relayOnlySender}
              hint={
                relayMbps === null
                  ? m.files.relayUnavailable
                  : format(m.files.relayOnlySenderHint, { limit })
              }
              disabled={relayMbps === null}
            />
          </RadioGroup>
          <div className="flex justify-end gap-2">
            <Button onPress={onClose} className={secondaryButtonClass}>
              {m.cancel}
            </Button>
            <Button
              isDisabled={file === null || how === null}
              onPress={() => {
                if (file === null || how === null) {
                  return;
                }
                sync.voice.offerFile(file, file.name, how === "direct", Number(validity));
                onClose();
              }}
              className={primaryButtonClass}
            >
              {m.files.send}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/**
 * Accepting a file: the receiver must choose how it travels among the ways its sender and the
 * server allow, even when there is only one, with each spelled out: direct, which could expose
 * their address to the sender, or the relay, capped to the server's limit.
 */
export function ReceiveFileDialog({
  offer,
  relayMbps,
  onClose,
}: {
  offer: OfferState;
  relayMbps: number | null;
  onClose: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const { locale } = useLocale();
  const sender = useUser(offer.from);
  const handle = sender === undefined ? m.unknownUser : handleOf(sender);
  const limit = useLimit(relayMbps);
  const modes = receiveModes(offer, relayMbps);
  const [mode, setMode] = useState<TransferMode | null>(null);
  return (
    <ModalOverlay
      isOpen
      isDismissable
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>{format(m.files.receiveHeading, { name: offer.name })}</DialogHeading>
          <p className="text-sm text-ink-muted">
            {format(m.files.receiveFrom, { size: formatSize(offer.size, locale), handle })}
          </p>
          <RadioGroup
            value={mode}
            onChange={(value) => {
              setMode(value === "relayOnly" ? "relayOnly" : "directPreferred");
            }}
            className="flex flex-col gap-2"
          >
            <Label className={labelClass}>{m.files.receiverChoice}</Label>
            {modes.includes("directPreferred") && (
              <Choice
                value="directPreferred"
                label={m.files.directPreferred}
                hint={format(
                  relayMbps === null
                    ? m.files.directPreferredNoRelayHint
                    : m.files.directPreferredHint,
                  { handle },
                )}
              />
            )}
            {modes.includes("relayOnly") && (
              <Choice
                value="relayOnly"
                label={m.files.relayOnly}
                hint={format(m.files.relayOnlyHint, { handle, limit })}
              />
            )}
          </RadioGroup>
          {modes.length === 1 && <p className="text-xs text-ink-muted">{m.files.onlyOption}</p>}
          {canChooseDestination() && (
            <p className="text-xs text-ink-muted">{m.files.chooseWhere}</p>
          )}
          <p className="text-xs text-ink-muted">{m.files.noResume}</p>
          <div className="flex justify-end gap-2">
            <Button onPress={onClose} className={secondaryButtonClass}>
              {m.cancel}
            </Button>
            <Button
              isDisabled={mode === null}
              onPress={() => {
                if (mode === null) {
                  return;
                }
                if (!canChooseDestination()) {
                  sync.voice.acceptOffer(offer.id, mode);
                  onClose();
                  return;
                }
                void chooseDestination(offer.name).then(
                  (sink) => {
                    // Closing the picker without choosing leaves the offer to accept later.
                    if (sink !== null) {
                      sync.voice.acceptOffer(offer.id, mode, sink);
                      onClose();
                    }
                  },
                  () => {
                    // Where the file cannot be opened, it is held and saved at the end instead.
                    sync.voice.acceptOffer(offer.id, mode);
                    onClose();
                  },
                );
              }}
              className={primaryButtonClass}
            >
              {m.files.receive}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
