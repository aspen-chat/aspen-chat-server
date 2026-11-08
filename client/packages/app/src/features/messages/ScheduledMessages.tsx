import type { ScheduledMessage } from "@aspen/protocol";
import { CaretRightIcon, ClockIcon, WarningCircleIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button, Dialog, Modal, ModalOverlay } from "react-aria-components";
import { useChannel, useScheduledMessages, useSync } from "@/api/hooks";
import { decodeCustomEmoji } from "@/features/emoji/customEmoji";
import {
  dialogClass,
  listModalClass,
  overlayClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { decodeTags } from "@/features/mentions/tags";
import { ScheduleTimeForm } from "@/features/messages/ScheduleMessageDialog";
import { SCHEDULED_TIME_FORMAT } from "@/features/messages/scheduleTimes";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

/**
 * A row's actions are not disabled while one of them runs, which `run` ignores another press
 * during: a disabled button drops focus out of the dialog, which then no longer closes on
 * Escape.
 */
const rowButtonClass = secondaryButtonClass + " aria-disabled:opacity-60";

/**
 * Above a channel's message box, what the caller has scheduled for later there: how many are
 * still to be sent and, apart and marked as a failure, how many could not be, each said only
 * when there are some. It is one button, which opens the list of them (`ScheduledList`); with
 * nothing scheduled there it is not drawn, though a list left open stays to say so. `onEdit` takes one back into the message box; a
 * place without a box to take it leaves that out. `standalone` draws it with the box's own
 * edge and padding, where no box follows to give them.
 */
export function ScheduledStrip({
  channelId,
  onEdit,
  standalone = false,
}: {
  channelId: string;
  onEdit?: (scheduledId: string) => Promise<void>;
  standalone?: boolean;
}) {
  const m = useMessages();
  const scheduled = useScheduledMessages(channelId);
  const [open, setOpen] = useState(false);
  // The list stays open over nothing once its last message goes, to say so, until it is closed.
  if (scheduled.length === 0 && !open) {
    return null;
  }
  const failed = scheduled.filter((message) => message.status === "failed").length;
  const waiting = scheduled.length - failed;
  return (
    <div
      className={standalone && scheduled.length > 0 ? "border-t border-line px-4 py-3" : undefined}
    >
      {scheduled.length > 0 && (
        <Button
          onPress={() => {
            setOpen(true);
          }}
          data-scheduled-strip
          className="motion-rise flex w-full flex-wrap items-center gap-x-3 gap-y-1 rounded-md border border-line bg-surface-sunken px-3 py-2 text-start text-sm text-ink-muted outline-none hover:bg-surface-hover pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <ClockIcon size={16} aria-hidden="true" className="shrink-0" />
          <span className="flex min-w-0 flex-1 flex-wrap items-center gap-x-3 gap-y-1">
            {waiting > 0 && <span>{format(m.scheduled.count, { count: waiting })}</span>}
            {failed > 0 && (
              <span className="flex items-center gap-1 text-danger" data-scheduled-failures>
                <WarningCircleIcon size={16} aria-hidden="true" className="shrink-0" />
                {format(m.scheduled.failedCount, { count: failed })}
              </span>
            )}
          </span>
          <CaretRightIcon size={14} aria-hidden="true" className="shrink-0 rtl:-scale-x-100" />
        </Button>
      )}
      <ModalOverlay isOpen={open} onOpenChange={setOpen} className={overlayClass} isDismissable>
        <Modal className={listModalClass}>
          <Dialog className={dialogClass}>
            {({ close }) => (
              <ScheduledList
                channelId={channelId}
                {...(onEdit === undefined
                  ? {}
                  : {
                      onEdit: async (scheduledId: string) => {
                        await onEdit(scheduledId);
                        close();
                      },
                    })}
              />
            )}
          </Dialog>
        </Modal>
      </ModalOverlay>
    </div>
  );
}

/**
 * The messages the caller scheduled in a channel, the soonest first, each with what it says,
 * when it is sent or why it could not be, and what may be done with it; giving one another
 * time is a second step of the same dialog.
 */
function ScheduledList({
  channelId,
  onEdit,
}: {
  channelId: string;
  onEdit?: (scheduledId: string) => Promise<void>;
}) {
  const m = useMessages();
  const sync = useSync();
  const scheduled = useScheduledMessages(channelId);
  const [changing, setChanging] = useState<string | null>(null);
  if (changing !== null && scheduled.some((message) => message.id === changing)) {
    return (
      <ScheduleTimeForm
        heading={m.scheduled.changeTime}
        submitLabel={m.scheduled.changeTime}
        pendingLabel={m.scheduled.changing}
        onBack={() => {
          setChanging(null);
        }}
        onSubmit={async (at) => {
          await sync.rescheduleMessage(changing, at);
          setChanging(null);
        }}
      />
    );
  }
  return (
    <>
      <DialogHeading>{m.scheduled.listHeading}</DialogHeading>
      {scheduled.length === 0 ? (
        <p className="text-sm text-ink-muted">{m.scheduled.none}</p>
      ) : (
        <ul aria-label={m.scheduled.listLabel} className="flex flex-col gap-2">
          {scheduled.map((message) => (
            <ScheduledRow
              key={message.id}
              message={message}
              onChangeTime={() => {
                setChanging(message.id);
              }}
              {...(onEdit === undefined ? {} : { onEdit })}
            />
          ))}
        </ul>
      )}
    </>
  );
}

function ScheduledRow({
  message,
  onChangeTime,
  onEdit,
}: {
  message: ScheduledMessage;
  onChangeTime: () => void;
  onEdit?: (scheduledId: string) => Promise<void>;
}) {
  const m = useMessages();
  const sync = useSync();
  const shown = useDateFormat(SCHEDULED_TIME_FORMAT);
  const channel = useChannel(message.channelId);
  const parent = useChannel(channel?.parentChannel ?? "");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const time = shown.format(new Date(message.sendAt));
  const files = message.attachments.length;
  // Tags and custom emoji read as they were written, as a reply's quote of a message does.
  const community = (channel?.parentChannel != null ? parent : channel)?.community ?? null;
  const roles = community === null ? [] : sync.store.roles(community);
  const text = decodeCustomEmoji(
    decodeTags(
      message.content,
      (id) => sync.store.user(id)?.name,
      (id) => roles.find((role) => role.id === id)?.name,
    ).text,
    community === null ? [] : sync.store.customEmoji(community),
  );

  function run(action: () => Promise<void>) {
    if (pending) {
      return;
    }
    setPending(true);
    setError(null);
    action()
      .catch((e: unknown) => {
        setError(e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        setPending(false);
      });
  }

  return (
    <li
      className="flex flex-col gap-2 rounded-md border border-line bg-surface px-3 py-2 text-sm"
      data-scheduled-message={message.id}
      data-scheduled-status={message.status}
    >
      {message.status === "failed" ? (
        <p className="flex items-start gap-1.5 text-danger">
          <WarningCircleIcon size={16} aria-hidden="true" className="mt-0.5 shrink-0" />
          <span className="min-w-0">
            {format(m.scheduled.failed, { reason: message.failure ?? "" })}
          </span>
        </p>
      ) : (
        <p className="flex items-start gap-1.5 text-ink-muted">
          <ClockIcon size={16} aria-hidden="true" className="mt-0.5 shrink-0" />
          <span className="flex min-w-0 flex-col">
            <span className="font-medium text-ink">
              {message.status === "waitingForMana"
                ? m.scheduled.waitingForMana
                : format(m.scheduled.sendsAt, { time })}
            </span>
            {message.status === "waitingForMana" && (
              <span className="text-xs">{m.scheduled.waitingForManaHint}</span>
            )}
          </span>
        </p>
      )}
      {text !== "" && (
        <p className="message-text line-clamp-4 wrap-anywhere whitespace-pre-wrap">{text}</p>
      )}
      <p className="flex flex-wrap gap-x-3 text-xs text-ink-faint">
        {message.status !== "scheduled" && (
          <span>{format(m.scheduled.scheduledFor, { time })}</span>
        )}
        {message.replyTo != null && <span>{m.scheduled.reply}</span>}
        {files > 0 && <span>{format(m.held.files, { count: files })}</span>}
      </p>
      <div className="flex flex-wrap gap-2">
        {message.status !== "waitingForMana" && (
          <Button
            aria-disabled={pending}
            onPress={() => {
              run(() => sync.sendScheduledNow(message.id));
            }}
            className={rowButtonClass}
          >
            {m.scheduled.sendNow}
          </Button>
        )}
        <Button
          aria-disabled={pending}
          onPress={() => {
            if (!pending) {
              onChangeTime();
            }
          }}
          className={rowButtonClass}
        >
          {m.scheduled.changeTime}
        </Button>
        {onEdit !== undefined && (
          <Button
            aria-disabled={pending}
            onPress={() => {
              run(() => onEdit(message.id));
            }}
            className={rowButtonClass}
          >
            {m.scheduled.edit}
          </Button>
        )}
        <Button
          aria-disabled={pending}
          onPress={() => {
            run(() => sync.deleteScheduled(message.id));
          }}
          className={rowButtonClass}
        >
          {m.scheduled.delete}
        </Button>
      </div>
      {error !== null && (
        <p role="alert" className="text-xs text-danger">
          {error}
        </p>
      )}
    </li>
  );
}
