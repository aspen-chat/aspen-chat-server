import { ApiProblemError } from "@aspen/protocol";
import { TOUCH_ONLY, useMediaQuery } from "@/features/layout/useMediaQuery";
import { useState, type KeyboardEvent } from "react";
import { Button, TextArea, TextField } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { decodeCustomEmoji } from "@/features/emoji/customEmoji";
import { useEmojiCompletion } from "@/features/emoji/useEmojiCompletion";
import { decodeTags } from "@/features/mentions/tags";
import { useTagging } from "@/features/mentions/useTagging";
import { MESSAGE_MAX_CHARS, messageLength } from "@/features/messages/messageLength";
import { MessageLengthNote } from "@/features/messages/MessageLengthNote";
import { useMessages } from "@/i18n/context";

/**
 * Replaces a message's text in place. Enter saves, Escape cancels, Shift+Enter breaks a line.
 * Its tags read as they were written (`@username`, `@Role`), and more can be picked as in the
 * message box.
 */
export function MessageEditor({
  messageId,
  channelId,
  initial,
  onDone,
}: {
  messageId: string;
  channelId: string;
  initial: string;
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
    const tags = decodeTags(
      initial,
      (id) => store.user(id)?.name,
      (id) => roles.find((role) => role.id === id)?.name,
    );
    return { ...tags, text: decodeCustomEmoji(tags.text, custom) };
  });
  const [draft, setDraft] = useState(decoded.text);
  const tagging = useTagging({ channelId, draft, setDraft, initialPicks: decoded.picks });
  const emoji = useEmojiCompletion({ channelId, draft, setDraft });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const content = emoji.encode(tagging.encode(draft.trim()));
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

  function onKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (tagging.onKeyDown(event) || emoji.onKeyDown(event)) {
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      onDone();
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
        onChange={setDraft}
        autoFocus
        className="relative"
      >
        <p role="status" className="sr-only">
          {emoji.open ? emoji.announcement : tagging.announcement}
        </p>
        {tagging.list}
        {emoji.list}
        <TextArea
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
          rows={1}
          onKeyDown={onKeyDown}
          className="message-box-text max-h-40 w-full resize-none rounded-md border border-line bg-surface-raised px-3 py-2 outline-none field-sizing-content focus:border-accent focus:ring-2 focus:ring-accent/30"
        />
      </TextField>
      <MessageLengthNote length={length} />
      {error !== null && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
      <div className="flex items-center gap-2 text-xs text-ink-faint">
        <span className="flex-1">{m.editingHint}</span>
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
    </div>
  );
}
