import { ApiProblemError, type Attachment } from "@aspen/protocol";
import { SmileyIcon } from "@phosphor-icons/react";
import { TOUCH_ONLY, useMediaQuery } from "@/features/layout/useMediaQuery";
import { useId, useRef, useState, type KeyboardEvent } from "react";
import { Button, TextArea, TextField } from "react-aria-components";
import { useAttachments, useSync } from "@/api/hooks";
import { encodeCustomEmoji } from "@/features/emoji/customEmoji";
import { EmojiKeyboard } from "@/features/emoji/EmojiKeyboard";
import { useEmojiCompletion } from "@/features/emoji/useEmojiCompletion";
import { decodePicked, encodePicked, trimPicked, usePickedText } from "@/features/messages/picks";
import { useTagging } from "@/features/mentions/useTagging";
import { MESSAGE_MAX_CHARS, messageLength } from "@/features/messages/messageLength";
import { MessageLengthNote } from "@/features/messages/MessageLengthNote";
import {
  AttachmentDescriptionButton,
  pendingControlClass,
  SpoilerToggle,
} from "@/features/messages/AttachmentDescription";
import { describable, isImageType } from "@/features/messages/images";
import { mediaUrl } from "@/features/layout/safeUrl";
import { useMessages } from "@/i18n/context";

/**
 * Replaces a message's text in place. Enter saves, Escape cancels, Shift+Enter breaks a line.
 * Its tags and custom emoji read as they were written (`@username`, `@Role`, `:name:`), each
 * a pick of what it named (`decodePicked`), so what is not changed is sent back as it was, and
 * more can be picked as in the
 * message box, whose emoji keyboard it has too, docked below it; Escape puts that away before
 * it cancels the edit. Its files stay as they are, but each can be described or marked a
 * spoiler, saved as soon as it is changed.
 */
export function MessageEditor({
  messageId,
  channelId,
  initial,
  attachments,
  onDone,
}: {
  messageId: string;
  channelId: string;
  initial: string;
  attachments: readonly string[];
  onDone: () => void;
}) {
  const m = useMessages();
  const touchOnly = useMediaQuery(TOUCH_ONLY);
  const sync = useSync();
  const [decoded] = useState(() => {
    const store = sync.store;
    const community = store.channel(channelId)?.community;
    const roles = community == null ? [] : store.roles(community);
    const custom = community == null ? [] : store.customEmoji(community);
    return decodePicked(initial, {
      usernameOf: (id) => store.user(id)?.name,
      roleNameOf: (id) => roles.find((role) => role.id === id)?.name,
      emojiNameOf: (id) => custom.find((e) => e.id.toLowerCase() === id.toLowerCase())?.name,
    });
  });
  const written = usePickedText(decoded);
  const draft = written.held.text;
  const tagging = useTagging({ channelId, draft, splice: written.splice });
  const emoji = useEmojiCompletion({ channelId, draft, splice: written.splice });
  const box = useRef<HTMLTextAreaElement>(null);
  const [keyboard, setKeyboard] = useState(false);
  const keyboardId = useId();
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const content = encodePicked(trimPicked(written.held), (text) =>
    encodeCustomEmoji(text, emoji.custom),
  );
  const length = messageLength(content);
  const tooLong = length > MESSAGE_MAX_CHARS;

  async function save() {
    if (content.length === 0 || tooLong || saving) {
      return;
    }
    if (content === initial) {
      onDone();
      return;
    }
    setSaving(true);
    setError(null);
    try {
      await sync.editMessage(messageId, content);
      onDone();
    } catch (e) {
      setError(e instanceof ApiProblemError ? e.message : String(e));
      setSaving(false);
    }
  }

  /** Puts the emoji keyboard away, and the focus back in the box, as the message box does. */
  function closeKeyboard() {
    setKeyboard(false);
    box.current?.focus();
  }

  function toggleKeyboard() {
    if (keyboard) {
      closeKeyboard();
    } else {
      setKeyboard(true);
      if (touchOnly) {
        box.current?.blur();
      }
    }
  }

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (tagging.onKeyDown(event) || emoji.onKeyDown(event)) {
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      if (keyboard) {
        setKeyboard(false);
      } else {
        onDone();
      }
    } else if (
      event.key === "Enter" &&
      !event.shiftKey &&
      !event.nativeEvent.isComposing &&
      !touchOnly
    ) {
      event.preventDefault();
      void save();
    }
  }

  return (
    <div className="mt-1 flex flex-col gap-1">
      <TextField
        aria-label={m.editMessageLabel}
        value={draft}
        onChange={(next) => {
          written.type(next, box.current?.selectionStart);
        }}
        autoFocus
        className="relative"
      >
        <p role="status" className="sr-only">
          {emoji.open ? emoji.announcement : tagging.announcement}
        </p>
        {tagging.list}
        {emoji.list}
        <TextArea
          ref={box}
          {...tagging.boxProps}
          {...(emoji.open
            ? {
                "aria-controls": emoji.listId,
                "aria-activedescendant": `${emoji.listId}-${String(emoji.current)}`,
              }
            : {})}
          onSelect={(event) => {
            tagging.boxProps.onSelect(event);
            emoji.follow(event);
          }}
          onKeyUp={(event) => {
            tagging.boxProps.onKeyUp(event);
            emoji.follow(event);
          }}
          onClick={(event) => {
            tagging.boxProps.onClick(event);
            emoji.follow(event);
          }}
          onFocus={() => {
            if (touchOnly && keyboard) {
              setKeyboard(false);
            }
          }}
          rows={1}
          onKeyDown={onKeyDown}
          className="message-box-text max-h-40 w-full resize-none rounded-md border border-line bg-surface-raised px-3 py-2 outline-none field-sizing-content focus:border-accent focus:ring-2 focus:ring-accent/30"
        />
      </TextField>
      <MessageLengthNote length={length} />
      {attachments.length > 0 && <EditedFiles messageId={messageId} ids={attachments} />}
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      <div className="flex items-center gap-2 text-xs text-ink-faint">
        <span className="flex-1">{m.editingHint}</span>
        <Button
          aria-label={m.emoji.insert}
          aria-expanded={keyboard}
          {...(keyboard ? { "aria-controls": keyboardId } : {})}
          onPress={toggleKeyboard}
          className={
            "rounded p-1 outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50" +
            (keyboard ? " text-accent" : "")
          }
        >
          <SmileyIcon size={18} aria-hidden="true" />
        </Button>
        <Button
          onPress={onDone}
          className="rounded px-2 py-0.5 outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          {m.cancel}
        </Button>
        <Button
          isDisabled={saving || tooLong || draft.trim().length === 0}
          onPress={() => {
            void save();
          }}
          className="rounded px-2 py-0.5 font-medium text-accent outline-none hover:underline disabled:opacity-60 focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          {m.save}
        </Button>
      </div>
      <EmojiKeyboard
        id={keyboardId}
        open={keyboard}
        communityId={emoji.communityId}
        onPick={(picked) => {
          emoji.insert(picked, box.current);
          if (!touchOnly) {
            box.current?.focus();
          }
        }}
        onClose={closeKeyboard}
      />
    </div>
  );
}

/**
 * The files of a message being edited, each with its spoiler mark and, for a picture or video,
 * its description, changed on the server as soon as they are changed here.
 */
function EditedFiles({ messageId, ids }: { messageId: string; ids: readonly string[] }) {
  const m = useMessages();
  const sync = useSync();
  const files = useAttachments(ids);
  const [error, setError] = useState<string | null>(null);
  function change(attachment: Attachment, change: { description?: string; spoiler?: boolean }) {
    setError(null);
    sync
      .retouchAttachment(messageId, attachment.id, {
        ...change,
        ...(change.description === undefined
          ? {}
          : { description: change.description === "" ? null : change.description }),
      })
      .catch((e: unknown) => {
        setError(e instanceof ApiProblemError ? e.message : String(e));
      });
  }
  return (
    <div className="flex flex-col gap-1">
      <ul aria-label={m.editedFiles} className="flex flex-wrap gap-2">
        {files.map(
          (file) =>
            file !== undefined && (
              <li
                key={file.id}
                className="flex items-center gap-1 rounded-md border border-line px-2 py-1 text-sm"
              >
                <span className="max-w-40 truncate">{file.fileName}</span>
                <SpoilerToggle
                  name={file.fileName}
                  spoiler={file.spoiler ?? false}
                  onChange={(spoiler) => {
                    change(file, { spoiler });
                  }}
                />
                {describable(file.mimeType) && (
                  <AttachmentDescriptionButton
                    name={file.fileName}
                    picture={
                      isImageType(file.mimeType) ? (mediaUrl(file.downloadUrl) ?? null) : null
                    }
                    description={file.description ?? ""}
                    onSave={(description) => {
                      change(file, { description });
                    }}
                    className={pendingControlClass}
                  />
                )}
              </li>
            ),
        )}
      </ul>
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
    </div>
  );
}
