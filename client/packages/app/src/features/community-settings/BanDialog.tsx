import { ApiProblemError } from "@aspen/protocol";
import { CaretDownIcon } from "@phosphor-icons/react";
import { useState } from "react";
import {
  Button,
  Dialog,
  Input,
  Label,
  ListBox,
  ListBoxItem,
  Modal,
  ModalOverlay,
  Popover,
  Select,
  SelectValue,
  TextField,
} from "react-aria-components";
import { useAccess, useCommunity, useSync } from "@/api/hooks";
import { alertClass, fieldClass, hintClass, inputClass, labelClass } from "@/features/auth/styles";
import {
  dangerButtonClass,
  dialogClass,
  modalClass,
  optionClass,
  overlayClass,
  selectButtonClass,
  selectPopoverClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { toast } from "@/features/layout/toast";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** How long a ban may be chosen to last, in seconds; `null` until lifted. */
const DURATIONS = [
  { id: "forever", seconds: null },
  { id: "hour", seconds: 3_600 },
  { id: "day", seconds: 86_400 },
  { id: "week", seconds: 7 * 86_400 },
  { id: "month", seconds: 30 * 86_400 },
] as const;

/** How far back a ban may delete the person's messages, in seconds; `null` keeps them. */
const DELETE_WINDOWS = [
  { id: "none", seconds: null },
  { id: "hour", seconds: 3_600 },
  { id: "day", seconds: 86_400 },
] as const;

/**
 * Bans a member: a reason they are shown when they try to come back, how long for, and, for a
 * banner who also holds Manage messages, whether their messages from the last hour or day go
 * with them. Done, it says so in a toast and closes.
 */
export function BanDialog({
  communityId,
  userId,
  name,
  isOpen,
  onOpenChange,
}: {
  communityId: string;
  userId: string;
  name: string;
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const community = useCommunity(communityId);
  const access = useAccess(communityId);
  const mayDelete = access?.has("manageMessages") ?? false;
  const [reason, setReason] = useState("");
  const [duration, setDuration] = useState<(typeof DURATIONS)[number]["id"]>("forever");
  const [window, setWindow] = useState<(typeof DELETE_WINDOWS)[number]["id"]>("none");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function ban() {
    if (busy) {
      return;
    }
    setBusy(true);
    setError(null);
    const seconds = DURATIONS.find((d) => d.id === duration)?.seconds ?? null;
    const deleting = mayDelete
      ? (DELETE_WINDOWS.find((w) => w.id === window)?.seconds ?? null)
      : null;
    try {
      const deleted = await sync.banMember(communityId, userId, {
        ...(reason.trim() === "" ? {} : { reason: reason.trim() }),
        ...(seconds === null ? {} : { durationSeconds: seconds }),
        ...(deleting === null ? {} : { deleteMessagesSeconds: deleting }),
      });
      toast(
        deleted > 0
          ? format(m.members.bannedAndDeleted, { name, count: String(deleted) })
          : format(m.members.banned, { name }),
      );
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog role="alertdialog" className={dialogClass}>
          <DialogHeading>{format(m.members.banHeading, { name })}</DialogHeading>
          <p className="text-sm text-ink-muted">
            {format(m.members.banHint, { community: community?.name ?? "" })}
          </p>
          <TextField value={reason} onChange={setReason} maxLength={512} className={fieldClass}>
            <Label className={labelClass}>{m.members.banReasonLabel}</Label>
            <Input className={inputClass} />
            <p className={hintClass}>{m.members.banReasonHint}</p>
          </TextField>
          <Select
            value={duration}
            onChange={(key) => {
              if (typeof key === "string" && DURATIONS.some((d) => d.id === key)) {
                setDuration(key as (typeof DURATIONS)[number]["id"]);
              }
            }}
            className="flex flex-col gap-1"
          >
            <Label className={labelClass}>{m.members.banDurationLabel}</Label>
            <Button className={selectButtonClass}>
              <SelectValue className="truncate" />
              <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
            </Button>
            <Popover className={selectPopoverClass}>
              <ListBox>
                {DURATIONS.map((d) => (
                  <ListBoxItem
                    key={d.id}
                    id={d.id}
                    textValue={m.members.banDurations[d.id]}
                    className={optionClass}
                  >
                    {m.members.banDurations[d.id]}
                  </ListBoxItem>
                ))}
              </ListBox>
            </Popover>
          </Select>
          {mayDelete && (
            <Select
              value={window}
              onChange={(key) => {
                if (typeof key === "string" && DELETE_WINDOWS.some((w) => w.id === key)) {
                  setWindow(key as (typeof DELETE_WINDOWS)[number]["id"]);
                }
              }}
              className="flex flex-col gap-1"
            >
              <Label className={labelClass}>{m.members.banDeleteLabel}</Label>
              <Button className={selectButtonClass}>
                <SelectValue className="truncate" />
                <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
              </Button>
              <Popover className={selectPopoverClass}>
                <ListBox>
                  {DELETE_WINDOWS.map((w) => (
                    <ListBoxItem
                      key={w.id}
                      id={w.id}
                      textValue={m.members.banDeleteOptions[w.id]}
                      className={optionClass}
                    >
                      {m.members.banDeleteOptions[w.id]}
                    </ListBoxItem>
                  ))}
                </ListBox>
              </Popover>
            </Select>
          )}
          {error !== null && (
            <p role="alert" className={alertClass}>
              {error}
            </p>
          )}
          <div className="flex justify-end">
            <Button
              isDisabled={busy}
              onPress={() => {
                void ban();
              }}
              className={dangerButtonClass}
            >
              {busy ? m.members.banning : m.members.banNow}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}
