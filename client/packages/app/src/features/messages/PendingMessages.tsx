import {
  ApiProblemError,
  heldInPlace,
  type HeldMessage,
  type ScheduledMessage,
  type Upload,
} from "@aspen/protocol";
import { CheckIcon, FileIcon, WarningCircleIcon } from "@phosphor-icons/react";
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Button, ProgressBar } from "react-aria-components";
import {
  useAttachment,
  useHeldMessages,
  useMe,
  useScheduledMessages,
  useStoredAttachment,
  useSync,
} from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { takeBackIn, useHasComposer } from "@/features/messages/composerTargets";
import { isImageType, pictureSrc } from "@/features/messages/images";
import type { ChannelHome } from "@/features/messages/links";
import { Markdown } from "@/features/messages/Markdown";
import { SCHEDULED_TIME_FORMAT } from "@/features/messages/scheduleTimes";
import { useNameColor } from "@/features/users/nameColor";
import { useNameIn } from "@/features/users/nameIn";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

/** What the caller sent in a channel that the server holds until its files have arrived. */
type Pending = HeldMessage | ScheduledMessage;

function isScheduled(pending: Pending): pending is ScheduledMessage {
  return "sendAt" in pending;
}

/**
 * What the caller sent in `channelId` before its files had finished uploading, in the order
 * sent: messages and commands the server holds until each file is confirmed and its preview
 * made (`heldInPlace`), and messages scheduled that are still `uploading`. One the server
 * dropped waits above the message box instead, with the rest it dropped (`HeldMessages`).
 */
function usePendingMessages(channelId: string): readonly Pending[] {
  const held = useHeldMessages(channelId);
  const scheduled = useScheduledMessages(channelId);
  return useMemo(
    () =>
      [
        ...held.flatMap((entry) =>
          entry.failure === null && heldInPlace(entry.message) ? [entry.message] : [],
        ),
        ...scheduled.filter((message) => message.status === "uploading"),
      ]
        // Their ids are made in the order they were sent.
        .sort((a, b) => a.id.localeCompare(b.id)),
    [held, scheduled],
  );
}

/**
 * At the foot of a channel's messages, what the caller sent there while its files were still
 * uploading, each drawn as the message it will be, faded, until the server posts it and it
 * arrives as any message does (`usePendingMessages`). The uploads go on whatever is shown
 * (`AspenSync.uploads`); each file says how far it has come, and one that failed can be sent
 * again. A message can be taken back into the box to be changed, or deleted.
 */
export function PendingMessages({ channelId, home }: { channelId: string; home: ChannelHome }) {
  const m = useMessages();
  const pending = usePendingMessages(channelId);
  if (pending.length === 0) {
    return null;
  }
  return (
    <ul aria-label={m.pending.label} className="flex flex-col gap-1">
      {pending.map((message) => (
        <PendingRow key={message.id} message={message} channelId={channelId} home={home} />
      ))}
    </ul>
  );
}

function PendingRow({
  message,
  channelId,
  home,
}: {
  message: Pending;
  channelId: string;
  home: ChannelHome;
}) {
  const m = useMessages();
  const sync = useSync();
  const me = useMe();
  const name = useNameIn(me ?? undefined, home.community);
  const nameColor = useNameColor(me?.id, home.community);
  const time = useDateFormat(SCHEDULED_TIME_FORMAT);
  const uploads = useSyncExternalStore(sync.uploads.subscribe, sync.uploads.all);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const boxOpen = useHasComposer(channelId);

  const scheduled = isScheduled(message);
  const uploading = scheduled ? message.status === "uploading" : message.waiting === "uploads";
  const mine = message.attachments.flatMap(
    (id) => uploads.find((u) => u.attachmentId === id) ?? [],
  );
  const failed = mine.filter((upload) => upload.state.kind === "failed");
  // One failure is said at a time: the first that can be put right by trying again, else the
  // first.
  const failure =
    failed.find((upload) => upload.state.kind === "failed" && upload.state.retriable) ?? failed[0];
  const retriable = failed.filter(
    (upload) => upload.state.kind === "failed" && upload.state.retriable,
  );
  // A command's text is the server's wording of it, which the box cannot read back; and a file
  // another device is sending cannot be taken into this one's box.
  const editable =
    (scheduled || message.kind === "message") &&
    boxOpen &&
    message.attachments.every(
      (id) => uploads.some((u) => u.attachmentId === id) || sync.store.attachment(id) !== undefined,
    );

  function run(action: () => Promise<void>) {
    setBusy(true);
    setError(null);
    action()
      .catch((e: unknown) => {
        setError(e instanceof ApiProblemError || e instanceof Error ? e.message : String(e));
      })
      .finally(() => {
        setBusy(false);
      });
  }

  const status =
    failed.length > 0
      ? null
      : scheduled
        ? format(m.pending.scheduling, { time: time.format(new Date(message.sendAt)) })
        : uploading
          ? m.pending.uploading
          : m.held.waiting;
  return (
    <li data-pending-message={message.id} className="flex flex-col gap-1 rounded px-2 py-1">
      <div className="not-yet-sent flex gap-3">
        <Avatar name={name ?? "?"} iconId={me?.icon} />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <span className="font-medium" style={{ color: nameColor }}>
              {name}
            </span>
            {!scheduled && message.kind === "command" && (
              <span className="text-xs text-ink-muted">{m.held.kind.command}</span>
            )}
            {status !== null && (
              <span role="status" className="text-xs text-ink-faint">
                {status}
              </span>
            )}
          </div>
          {message.content !== "" &&
            (!scheduled && message.kind === "command" ? (
              <p className="message-body whitespace-pre-wrap">{message.content}</p>
            ) : (
              <Markdown content={message.content} communityId={home.community} />
            ))}
          {message.attachments.length > 0 && (
            <ul aria-label={m.pendingAttachmentsLabel} className="mt-1 flex flex-wrap gap-2">
              {message.attachments.map((id) => (
                <PendingFile
                  key={id}
                  attachmentId={id}
                  upload={uploads.find((u) => u.attachmentId === id)}
                  uploading={uploading}
                />
              ))}
            </ul>
          )}
        </div>
      </div>
      {failure?.state.kind === "failed" && (
        <p role="alert" className="ps-14 text-sm text-danger">
          <WarningCircleIcon
            size={16}
            aria-hidden="true"
            className="me-1 inline align-text-bottom"
          />
          {format(m.pending.failed[failure.state.cause], { name: failure.name })}
          {failure.state.reason !== null && <span className="block">{failure.state.reason}</span>}
        </p>
      )}
      {error !== null && (
        <p role="alert" className="ps-14 text-sm text-danger">
          {error}
        </p>
      )}
      <div className="flex flex-wrap gap-2 ps-14">
        {retriable.length > 0 && (
          <Button
            isDisabled={busy}
            onPress={() => {
              for (const upload of retriable) {
                sync.uploads.retry(upload.key);
              }
            }}
            className={secondaryButtonClass}
          >
            {m.pending.retry}
          </Button>
        )}
        {editable && (
          <Button
            isDisabled={busy}
            onPress={() => {
              run(() => takeBackIn(channelId, message));
            }}
            className={secondaryButtonClass}
          >
            {m.pending.edit}
          </Button>
        )}
        <Button
          isDisabled={busy}
          onPress={() => {
            run(async () => {
              await (scheduled ? sync.deleteScheduled(message.id) : sync.cancelHeld(message.id));
              // Files of it still on their way would arrive for nothing.
              sync.uploads.discardAttachments(message.attachments);
            });
          }}
          className={secondaryButtonClass}
        >
          {m.pending.delete}
        </Button>
      </div>
    </li>
  );
}

/**
 * One file of a message not yet sent: on its way from this device, with how far it has come or
 * that it failed; arrived; or, when neither is known here, on its way from another device.
 */
function PendingFile({
  attachmentId,
  upload,
  uploading,
}: {
  attachmentId: string;
  upload: Upload | undefined;
  /** Whether the message still waits for files: once it does not, each has a record to read. */
  uploading: boolean;
}) {
  return upload !== undefined ? (
    <UploadingFile upload={upload} />
  ) : uploading ? (
    <PerhapsElsewhere attachmentId={attachmentId} />
  ) : (
    <ArrivedFile attachmentId={attachmentId} />
  );
}

const chipClass = "flex items-center gap-2 rounded-md border border-line p-1.5 text-sm";
const thumbnailClass = "h-14 w-14 rounded bg-surface-sunken object-cover";

function UploadingFile({ upload }: { upload: Upload }) {
  const m = useMessages();
  // The picture as this device holds it, until it has arrived and has an address of its own:
  // an address for its bytes, which holds them until it is given up.
  const picture = useRef<HTMLImageElement>(null);
  const { file, mimeType } = upload;
  const pictured = file !== null && isImageType(mimeType);
  useEffect(() => {
    if (file === null || picture.current === null) {
      return;
    }
    const url = URL.createObjectURL(file);
    picture.current.src = url;
    return () => {
      URL.revokeObjectURL(url);
    };
  }, [file, pictured]);
  const failed = upload.state.kind === "failed";
  return (
    <li className={chipClass + (failed ? " border-danger text-danger" : "")}>
      {pictured ? (
        <img ref={picture} alt="" className={thumbnailClass} />
      ) : (
        <FileIcon size={20} aria-hidden="true" className="text-ink-muted" />
      )}
      <span className="flex min-w-0 flex-col">
        <span className="max-w-40 truncate">{upload.name}</span>
        {upload.state.kind === "uploading" ? (
          <ProgressBar
            aria-label={format(m.uploadingFile, { name: upload.name })}
            value={upload.state.sent * 100}
            className="mt-1 w-full"
          >
            {({ percentage }) => (
              <span className="block h-1 w-full overflow-hidden rounded-full bg-line">
                <span
                  className="forced-fill block h-full rounded-full bg-accent transition-[width]"
                  style={{ width: `${String(percentage ?? 0)}%` }}
                />
              </span>
            )}
          </ProgressBar>
        ) : (
          <span className="text-xs">{m.uploadFailed}</span>
        )}
      </span>
    </li>
  );
}

/** A file not on its way from here: arrived, if the store has its record, else another device's. */
function PerhapsElsewhere({ attachmentId }: { attachmentId: string }) {
  const m = useMessages();
  const arrived = useStoredAttachment(attachmentId);
  if (arrived !== undefined) {
    return <ArrivedFile attachmentId={attachmentId} />;
  }
  return (
    <li className={chipClass}>
      <FileIcon size={20} aria-hidden="true" className="text-ink-muted" />
      <span className="text-xs text-ink-muted">{m.pending.elsewhere}</span>
    </li>
  );
}

function ArrivedFile({ attachmentId }: { attachmentId: string }) {
  const m = useMessages();
  const attachment = useAttachment(attachmentId);
  const picture = attachment === undefined ? undefined : pictureSrc(attachment, "preview");
  return (
    <li className={chipClass}>
      {picture !== undefined ? (
        <img src={picture} alt="" className={thumbnailClass} />
      ) : (
        <FileIcon size={20} aria-hidden="true" className="text-ink-muted" />
      )}
      <span className="flex min-w-0 flex-col">
        <span className="max-w-40 truncate">{attachment?.fileName}</span>
        <span className="flex items-center gap-1 text-xs text-ink-muted">
          <CheckIcon size={12} aria-hidden="true" />
          {m.pending.uploaded}
        </span>
      </span>
    </li>
  );
}
