import { ApiProblemError, type Attachment } from "@aspen/protocol";
import { FileIcon, PaperclipIcon, XIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState, type ChangeEvent, type KeyboardEvent } from "react";
import { Button, TextArea, TextField } from "react-aria-components";
import { useChannel, useChannelAccess, useSync } from "@/api/hooks";
import { isImageType } from "@/features/messages/images";
import { CreatePollDialog } from "@/features/messages/CreatePollDialog";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** A file chosen for the next message, at whatever stage its upload has reached. */
interface Pending {
  key: number;
  name: string;
  /** An object URL for the file's own bytes when it is an image, shown as a thumbnail. */
  thumbnail: string | null;
  state:
    | { kind: "uploading" }
    | { kind: "ready"; attachment: Attachment }
    | { kind: "failed"; reason: string };
}

let nextKey = 1;

const toolButtonClass =
  "rounded-md border border-line p-2.5 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * The message box. Enter sends, Shift+Enter breaks a line. Files chosen with the attach button
 * upload at once and are sent with the next message; a message may be files alone. In a thread,
 * `echoTarget` names the parent channel and a checkbox offers to show the reply there too; it
 * clears after each message. Only what the caller may do here is offered: without sending (or,
 * in a thread, sending in threads) the box gives way to a note saying so.
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
  const sync = useSync();
  const [draft, setDraft] = useState("");
  const [echo, setEcho] = useState(false);
  const [pending, setPending] = useState<Pending[]>([]);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const channel = useChannel(channelId);
  const permissions = useChannelAccess(channelId);
  const mayPost = permissions.has(channel?.ty === "thread" ? "sendInThreads" : "sendMessages");

  const uploading = pending.some((p) => p.state.kind === "uploading");
  const readyIds = pending.flatMap((p) =>
    p.state.kind === "ready" ? [p.state.attachment.id] : [],
  );
  const canSend = !sending && !uploading && (draft.trim().length > 0 || readyIds.length > 0);

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
        { key, name: file.name, thumbnail, state: { kind: "uploading" } },
      ]);
      sync.uploadAttachment(file).then(
        (attachment) => {
          update({ kind: "ready", attachment });
        },
        (e: unknown) => {
          update({ kind: "failed", reason: e instanceof ApiProblemError ? e.message : String(e) });
        },
      );
    }
  }

  async function send() {
    if (!canSend) {
      return;
    }
    setSending(true);
    setError(null);
    try {
      await sync.sendMessage(channelId, draft.trim(), readyIds, {
        echoToParent: echoTarget !== undefined && echo,
      });
      setDraft("");
      setPending([]);
      setEcho(false);
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
    } finally {
      setSending(false);
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
      event.preventDefault();
      void send();
    }
  }

  if (!mayPost) {
    return (
      <p className="border-t border-line px-4 py-4 text-sm text-ink-muted">{m.cannotSendHere}</p>
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
                "relative flex items-center gap-2 rounded-md border p-1.5 pr-8 text-sm " +
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
                  <span className="text-xs text-ink-faint">{m.uploading}</span>
                )}
                {p.state.kind === "failed" && (
                  <span className="text-xs" title={p.state.reason}>
                    {m.uploadFailed}
                  </span>
                )}
              </span>
              <Button
                aria-label={format(m.removeAttachment, { name: p.name })}
                onPress={() => {
                  setPending((list) => list.filter((other) => other.key !== p.key));
                }}
                className="absolute top-1 right-1 rounded p-0.5 text-ink-faint outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
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
        <TextField aria-label={m.messageLabel} value={draft} onChange={setDraft} className="flex-1">
          <TextArea
            placeholder={placeholder}
            rows={1}
            onKeyDown={onKeyDown}
            className="max-h-40 w-full resize-none rounded-md border border-line bg-surface-raised px-3 py-2 outline-none field-sizing-content focus:border-accent focus:ring-2 focus:ring-accent/30"
          />
        </TextField>
        <Button
          type="submit"
          isDisabled={!canSend}
          className="rounded-md bg-accent px-4 py-2 font-medium text-accent-contrast outline-none hover:bg-accent-strong pressed:opacity-80 disabled:opacity-60 focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          {m.send}
        </Button>
      </div>
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
