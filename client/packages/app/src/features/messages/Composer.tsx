import { ApiProblemError, type Attachment } from "@aspen/protocol";
import {
  ChartBarIcon,
  FileIcon,
  PaperclipIcon,
  PaperPlaneRightIcon,
  PlusIcon,
  XIcon,
} from "@phosphor-icons/react";
import { useEffect, useRef, useState, type ChangeEvent, type KeyboardEvent } from "react";
import {
  Button,
  Menu,
  MenuItem,
  MenuTrigger,
  Popover,
  ProgressBar,
  TextArea,
  TextField,
} from "react-aria-components";
import { useEmojiCompletion } from "@/features/emoji/useEmojiCompletion";
import { useTagging } from "@/features/mentions/useTagging";
import { useCommandLine } from "@/features/commands/useCommandLine";
import {
  useBlockedDmPeer,
  useChannel,
  useChannelAccess,
  useMe,
  useSync,
  useSystemDmPeer,
  useUser,
} from "@/api/hooks";
import { describable, isImageType } from "@/features/messages/images";
import { AttachmentDescriptionButton } from "@/features/messages/AttachmentDescription";
import { HeldMessages } from "@/features/messages/HeldMessages";
import { CreatePollDialog, CreatePollModal } from "@/features/messages/CreatePollDialog";
import { MEDIUM_SCREEN, useMediaQuery, TOUCH_ONLY } from "@/features/layout/useMediaQuery";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { displayNameOf } from "@/features/users/profile";
import { measurePicture } from "@/features/media/measurePicture";
import { noteDraft, readDraft, writeDraft } from "@/features/messages/drafts";
import { mediaUrl } from "@/features/layout/safeUrl";
import { MESSAGE_MAX_CHARS, messageLength } from "@/features/messages/messageLength";
import { MessageLengthNote } from "@/features/messages/MessageLengthNote";

/** A file chosen for the next message, at whatever stage its upload has reached. */
interface Pending {
  key: number;
  name: string;
  mimeType: string;
  /**
   * What it shows, as the user described it here; the attachment's own record has it once it
   * has reached the server (`describe`).
   */
  description: string;
  /** An object URL for the file's own bytes when it is an image, shown as a thumbnail. */
  thumbnail: string | null;
  state:
    /** `sent` is how much of the file has reached storage, 0 to 1. */
    | { kind: "uploading"; sent: number }
    | { kind: "ready"; attachment: Attachment }
    | { kind: "failed"; reason: string };
}

let nextKey = 1;

/** How long typing pauses before the draft is kept. */
const DRAFT_SAVE_DELAY_MS = 400;

const menuItemClass =
  "flex cursor-default items-center gap-2 rounded px-2 py-1.5 text-sm outline-none " +
  "focus:bg-surface-hover pointer-coarse:py-2.5";

const toolButtonClass =
  "shrink-0 rounded-md border border-line p-2.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * The message box. Enter sends, Shift+Enter breaks a line. Files chosen with the attach button
 * upload at once and are sent with the next message; a message may be files alone. In a thread,
 * `echoTarget` names the parent channel and a checkbox offers to show the reply there too; it
 * clears after each message. Only what the caller may do here is offered: without sending (or,
 * in a thread, sending in threads) the box gives way to a note saying so, in a DM with
 * someone the caller blocked, to a note offering to unblock them, and in the system account's
 * DM, to a note that its notices are not answered.
 */
export function Composer({
  channelId,
  placeholder,
  echoTarget,
}: {
  channelId: string;
  placeholder: string;
  echoTarget?: string;
}) {
  const m = useMessages();
  const touchOnly = useMediaQuery(TOUCH_ONLY);
  const sync = useSync();
  const me = useMe();
  // What was written here and not sent, kept on this device (`drafts.ts`). The box is made
  // afresh for each channel, so it reads its own.
  const [saved] = useState(() => (me === null ? null : readDraft(me.id, channelId)));
  const [draft, setDraft] = useState(saved?.text ?? "");
  const [echo, setEcho] = useState(saved?.echo ?? false);
  const [pending, setPending] = useState<Pending[]>(() =>
    (saved?.attachments ?? []).map((attachment) => ({
      key: nextKey++,
      name: attachment.fileName,
      mimeType: attachment.mimeType,
      description: attachment.description ?? "",
      thumbnail: isImageType(attachment.mimeType)
        ? (mediaUrl(attachment.downloadUrl) ?? null)
        : null,
      state: { kind: "ready", attachment },
    })),
  );
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  // Descriptions given while a file was still uploading, by its key, for the upload to give
  // the server once it is done.
  const describedEarly = useRef(new Map<number, string>());
  const channel = useChannel(channelId);
  const permissions = useChannelAccess(channelId);
  const mayPost = permissions.has(channel?.ty === "thread" ? "sendInThreads" : "sendMessages");
  const blockedPeer = useBlockedDmPeer(channelId);
  const systemPeer = useSystemDmPeer(channelId);
  const commands = useCommandLine({ channelId, draft, setDraft });
  const wide = useMediaQuery(MEDIUM_SCREEN);
  const [polling, setPolling] = useState(false);
  const tagging = useTagging({
    channelId,
    draft,
    setDraft,
    off: commands.active,
    initialPicks: saved?.picks ?? [],
  });
  const emoji = useEmojiCompletion({ channelId, draft, setDraft, off: commands.active });

  // The draft is noted as it changes, kept a moment after, and kept at once when the box goes
  // (another channel, a notification opened) or the page is hidden or left.
  const keep = useRef<(() => void) | null>(null);
  useEffect(() => {
    if (me === null) {
      keep.current = null;
      return;
    }
    const current = {
      text: draft,
      picks: tagging.picks,
      attachments: pending.flatMap((p) => (p.state.kind === "ready" ? [p.state.attachment] : [])),
      echo,
    };
    noteDraft(me.id, channelId, current);
    keep.current = () => {
      writeDraft(me.id, channelId, current);
    };
  });
  useEffect(() => {
    const timer = setTimeout(() => keep.current?.(), DRAFT_SAVE_DELAY_MS);
    return () => {
      clearTimeout(timer);
    };
  }, [draft, pending, echo, tagging.picks]);
  useEffect(() => {
    const flush = () => keep.current?.();
    window.addEventListener("pagehide", flush);
    document.addEventListener("visibilitychange", flush);
    return () => {
      window.removeEventListener("pagehide", flush);
      document.removeEventListener("visibilitychange", flush);
      flush();
    };
  }, []);

  const uploading = pending.some((p) => p.state.kind === "uploading");
  const readyIds = pending.flatMap((p) =>
    p.state.kind === "ready" ? [p.state.attachment.id] : [],
  );
  // Counted as sent, tags and custom emoji written out as their references.
  const length = messageLength(emoji.encode(tagging.encode(draft.trim())));
  const canSend =
    !sending &&
    !uploading &&
    length <= MESSAGE_MAX_CHARS &&
    (draft.trim().length > 0 || readyIds.length > 0);

  // Object URLs hold their file's bytes until revoked; release them once no chip shows them.
  const thumbnails = useRef(new Set<string>());
  useEffect(() => {
    const shown = new Set(pending.flatMap((p) => (p.thumbnail === null ? [] : [p.thumbnail])));
    for (const url of thumbnails.current) {
      if (!shown.has(url)) {
        URL.revokeObjectURL(url);
        thumbnails.current.delete(url);
      }
    }
  }, [pending]);
  useEffect(() => {
    const held = thumbnails.current;
    return () => {
      for (const url of held) {
        URL.revokeObjectURL(url);
      }
      held.clear();
    };
  }, []);

  function attach(event: ChangeEvent<HTMLInputElement>) {
    const files = Array.from(event.target.files ?? []);
    event.target.value = "";
    for (const file of files) {
      const key = nextKey++;
      let thumbnail: string | null = null;
      if (isImageType(file.type)) {
        thumbnail = URL.createObjectURL(file);
        thumbnails.current.add(thumbnail);
      }
      const update = (state: Pending["state"]) => {
        setPending((list) => list.map((p) => (p.key === key ? { ...p, state } : p)));
      };
      setPending((list) => [
        ...list,
        {
          key,
          name: file.name,
          mimeType: file.type,
          description: "",
          thumbnail,
          state: { kind: "uploading", sent: 0 },
        },
      ]);
      measurePicture(file)
        .then((size) =>
          sync.uploadAttachment(file, size, (sent, total) => {
            setPending((list) =>
              list.map((p) =>
                p.key === key && p.state.kind === "uploading"
                  ? { ...p, state: { kind: "uploading", sent: total > 0 ? sent / total : 0 } }
                  : p,
              ),
            );
          }),
        )
        .then(async (uploaded) => {
          const early = describedEarly.current.get(key);
          describedEarly.current.delete(key);
          // A description that fails to reach the server now is given again on sending.
          return early === undefined
            ? uploaded
            : await sync.describeAttachment(uploaded.id, early || null).catch(() => uploaded);
        })
        .then(
          (attachment) => {
            update({ kind: "ready", attachment });
          },
          (e: unknown) => {
            update({
              kind: "failed",
              reason: e instanceof ApiProblemError ? e.message : String(e),
            });
          },
        );
    }
  }

  /**
   * Keeps what a file shows as the user described it, and gives it to the server once the file
   * is there; a file still uploading gives it when the upload is done.
   */
  function describe(key: number, description: string) {
    const item = pending.find((p) => p.key === key);
    setPending((list) => list.map((p) => (p.key === key ? { ...p, description } : p)));
    if (item?.state.kind !== "ready") {
      describedEarly.current.set(key, description);
      return;
    }
    sync.describeAttachment(item.state.attachment.id, description || null).then(
      (attachment) => {
        setPending((list) =>
          list.map((p) =>
            p.key === key && p.state.kind === "ready"
              ? { ...p, state: { kind: "ready", attachment } }
              : p,
          ),
        );
      },
      (e: unknown) => {
        setError(
          format(m.describeFailed, {
            reason: e instanceof ApiProblemError ? e.message : String(e),
          }),
        );
      },
    );
  }

  async function send() {
    if (!canSend) {
      return;
    }
    setSending(true);
    setError(null);
    try {
      const text = draft.trim();
      // A line beginning with `/` is read against the commands here, which it may be sent
      // before they have been read.
      if (text.startsWith("/") && sync.store.commands(channelId) === undefined) {
        await sync.loadCommands(channelId).catch(() => undefined);
      }
      // Descriptions that have not reached the server yet (one given just as its upload ended,
      // or one whose saving failed) go before the message that carries their files.
      await Promise.all(
        pending.flatMap((p) =>
          p.state.kind === "ready" && p.description !== (p.state.attachment.description ?? "")
            ? [sync.describeAttachment(p.state.attachment.id, p.description || null)]
            : [],
        ),
      );
      const prepared = commands.prepare(text, readyIds);
      if (prepared.kind === "refused") {
        setError(prepared.reason);
        return;
      }
      if (prepared.kind === "command") {
        await sync.invokeCommand(channelId, prepared.invocation);
      } else {
        await sync.sendMessage(channelId, emoji.encode(tagging.encode(text)), readyIds, {
          echoToParent: echoTarget !== undefined && echo,
        });
      }
      setDraft("");
      tagging.reset();
      commands.reset();
      if (me !== null) {
        writeDraft(me.id, channelId, null);
      }
      setPending([]);
      setEcho(false);
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    } finally {
      setSending(false);
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (commands.onKeyDown(event) || tagging.onKeyDown(event) || emoji.onKeyDown(event)) {
      return;
    }
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing && !touchOnly) {
      event.preventDefault();
      void send();
    }
  }

  if (!mayPost) {
    if (systemPeer !== null) {
      return <SystemNote userId={systemPeer} />;
    }
    return blockedPeer === null ? (
      <p className="border-t border-line px-4 py-4 text-sm text-ink-muted">{m.cannotSendHere}</p>
    ) : (
      <BlockedNote userId={blockedPeer} />
    );
  }

  return (
    <form
      onSubmit={(event) => {
        event.preventDefault();
        void send();
      }}
      className="flex flex-col gap-2 border-t border-line px-4 py-3"
    >
      <HeldMessages channelId={channelId} />
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      {pending.length > 0 && (
        <ul aria-label={m.pendingAttachmentsLabel} className="flex flex-wrap gap-2">
          {pending.map((p) => (
            <li
              key={p.key}
              className={
                "motion-grow relative flex items-center gap-2 rounded-md border p-1.5 pe-8 text-sm " +
                (p.state.kind === "failed" ? "border-danger text-danger" : "border-line")
              }
            >
              {p.thumbnail !== null ? (
                <img
                  src={p.thumbnail}
                  alt=""
                  className="h-14 w-14 rounded object-cover bg-surface-sunken"
                />
              ) : (
                <FileIcon size={20} aria-hidden="true" className="text-ink-muted" />
              )}
              <span className="flex min-w-0 flex-col">
                <span className="max-w-40 truncate">{p.name}</span>
                {p.state.kind === "uploading" && (
                  <ProgressBar
                    aria-label={format(m.uploadingFile, { name: p.name })}
                    value={p.state.sent * 100}
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
                )}
                {p.state.kind === "failed" && (
                  <span className="text-xs" title={p.state.reason}>
                    {m.uploadFailed}
                  </span>
                )}
              </span>
              {describable(p.mimeType) && p.state.kind !== "failed" && (
                <AttachmentDescriptionButton
                  name={p.name}
                  description={p.description}
                  onSave={(description) => {
                    describe(p.key, description);
                  }}
                  className="absolute end-1 bottom-1 rounded p-0.5 text-ink-faint outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
                />
              )}
              <Button
                aria-label={format(m.removeAttachment, { name: p.name })}
                onPress={() => {
                  setPending((list) => list.filter((other) => other.key !== p.key));
                }}
                className="absolute top-1 end-1 rounded p-0.5 text-ink-faint outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
              >
                <XIcon size={14} aria-hidden="true" />
              </Button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex items-end gap-2">
        <input
          ref={fileInput}
          type="file"
          multiple
          hidden
          onChange={attach}
          aria-hidden="true"
          tabIndex={-1}
        />
        {wide ? (
          <>
            {permissions.has("attachFiles") && (
              <Tooltip text={m.attachFile}>
                <Button
                  aria-label={m.attachFile}
                  onPress={() => fileInput.current?.click()}
                  className={toolButtonClass}
                >
                  <PaperclipIcon size={20} aria-hidden="true" />
                </Button>
              </Tooltip>
            )}
            {permissions.has("createPolls") && (
              <CreatePollDialog channelId={channelId} triggerClassName={toolButtonClass} />
            )}
          </>
        ) : (
          (permissions.has("attachFiles") || permissions.has("createPolls")) && (
            // On a narrow screen the box's other controls share one button, leaving it room.
            <MenuTrigger>
              <Button aria-label={m.composerMore} className={toolButtonClass}>
                <PlusIcon size={20} aria-hidden="true" />
              </Button>
              <Popover className="rounded-md border border-line bg-surface-raised p-1 shadow-lg">
                <Menu
                  className="min-w-44 outline-none"
                  onAction={(key) => {
                    if (key === "attach") {
                      fileInput.current?.click();
                    } else if (key === "poll") {
                      setPolling(true);
                    }
                  }}
                >
                  {permissions.has("attachFiles") && (
                    <MenuItem id="attach" textValue={m.attachFile} className={menuItemClass}>
                      <PaperclipIcon size={18} aria-hidden="true" />
                      {m.attachFile}
                    </MenuItem>
                  )}
                  {permissions.has("createPolls") && (
                    <MenuItem id="poll" textValue={m.poll.open} className={menuItemClass}>
                      <ChartBarIcon size={18} aria-hidden="true" />
                      {m.poll.open}
                    </MenuItem>
                  )}
                </Menu>
              </Popover>
              <CreatePollModal channelId={channelId} isOpen={polling} onOpenChange={setPolling} />
            </MenuTrigger>
          )
        )}
        <TextField
          aria-label={m.messageLabel}
          value={draft}
          onChange={setDraft}
          className="message-text relative min-w-0 flex-1"
        >
          <p role="status" className="sr-only">
            {commands.active
              ? commands.announcement
              : emoji.open
                ? emoji.announcement
                : tagging.announcement}
          </p>
          {tagging.list}
          {emoji.list}
          {commands.list}
          <TextArea
            {...tagging.boxProps}
            {...(emoji.open
              ? {
                  "aria-controls": emoji.listId,
                  "aria-activedescendant": `${emoji.listId}-${String(emoji.current)}`,
                }
              : {})}
            {...(commands.active ? commands.aria : {})}
            onSelect={(event) => {
              tagging.boxProps.onSelect(event);
              emoji.follow(event);
              commands.follow(event);
            }}
            onKeyUp={(event) => {
              tagging.boxProps.onKeyUp(event);
              emoji.follow(event);
              commands.follow(event);
            }}
            onClick={(event) => {
              tagging.boxProps.onClick(event);
              emoji.follow(event);
              commands.follow(event);
            }}
            aria-placeholder={placeholder}
            rows={1}
            onKeyDown={onKeyDown}
            className="message-box-text block max-h-40 w-full max-w-full min-w-0 resize-none wrap-anywhere rounded-md border border-line bg-surface-raised px-3 py-2 outline-none field-sizing-content focus:border-accent focus:ring-2 focus:ring-accent/30"
          />
          {draft === "" && (
            // The box's own placeholder would wrap, and grow the box, where it is too long for
            // one line; this one is cut short with an ellipsis instead.
            <span
              aria-hidden="true"
              className="message-box-text pointer-events-none absolute inset-x-[13px] bottom-[9px] truncate text-ink-faint"
            >
              {placeholder}
            </span>
          )}
        </TextField>
        <Tooltip text={m.send}>
          <Button
            type="submit"
            aria-label={m.send}
            isDisabled={!canSend}
            className="shrink-0 rounded-md border border-transparent bg-accent p-2.5 text-accent-contrast outline-none hover:bg-accent-strong pressed:opacity-80 disabled:opacity-60 focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <PaperPlaneRightIcon size={20} aria-hidden="true" className="rtl:-scale-x-100" />
          </Button>
        </Tooltip>
      </div>
      <MessageLengthNote length={length} />
      {echoTarget !== undefined && permissions.has("sendMessages") && (
        <label className="mt-2 flex w-fit items-center gap-2 text-sm text-ink-muted">
          <input
            type="checkbox"
            checked={echo}
            onChange={(event) => {
              setEcho(event.target.checked);
            }}
            className="h-4 w-4 accent-accent"
          />
          {format(m.threads.echoToParent, { channel: echoTarget })}
        </label>
      )}
    </form>
  );
}

/** In place of the box in the system account's DM: its notices are read, not answered. */
function SystemNote({ userId }: { userId: string }) {
  const m = useMessages();
  const user = useUser(userId);
  const name = user === undefined ? m.unknownUser : displayNameOf(user);
  return (
    <p className="border-t border-line px-4 py-4 text-sm text-ink-muted">
      {format(m.system.readOnly, { name })}
    </p>
  );
}

/** In place of the box in a DM with someone the reader blocked: why, and a way to unblock. */
function BlockedNote({ userId }: { userId: string }) {
  const m = useMessages();
  const sync = useSync();
  const user = useUser(userId);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const name = user === undefined ? m.unknownUser : displayNameOf(user);
  return (
    <div className="flex flex-wrap items-center gap-3 border-t border-line px-4 py-3 text-sm text-ink-muted">
      <span className="min-w-0 flex-1">{format(m.blocking.dmBlocked, { name })}</span>
      <Button
        isDisabled={pending}
        onPress={() => {
          setPending(true);
          setError(null);
          sync.unblockUser(userId).catch((failure: unknown) => {
            setError(failure instanceof Error ? failure.message : String(failure));
            setPending(false);
          });
        }}
        className={secondaryButtonClass}
      >
        {m.blocking.unblock}
      </Button>
      {error !== null && (
        <p role="alert" className="w-full text-xs text-danger">
          {error}
        </p>
      )}
    </div>
  );
}
