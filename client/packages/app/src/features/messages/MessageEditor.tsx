import { ApiProblemError } from "@aspen/protocol";
import { useState, type KeyboardEvent } from "react";
import { Button, TextArea, TextField } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { decodeTags } from "@/features/mentions/tags";
import { useTagging } from "@/features/mentions/useTagging";
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
  const sync = useSync();
  const [decoded] = useState(() => {
    const store = sync.store;
    const community = store.channel(channelId)?.community;
    const roles = community == null ? [] : store.roles(community);
    return decodeTags(
      initial,
      (id) => store.user(id)?.name,
      (id) => roles.find((role) => role.id === id)?.name,
    );
  });
  const [draft, setDraft] = useState(decoded.text);
  const tagging = useTagging({ channelId, draft, setDraft, initialPicks: decoded.picks });
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function save() {
    const content = tagging.encode(draft.trim());
    if (content.length === 0 || saving) {
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
    if (tagging.onKeyDown(event)) {
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      onDone();
    } else if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
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
          {tagging.announcement}
        </p>
        {tagging.list}
        <TextArea
          {...tagging.boxProps}
          rows={1}
          onKeyDown={onKeyDown}
          className="max-h-40 w-full resize-none rounded-md border border-line bg-surface-raised px-3 py-2 outline-none field-sizing-content focus:border-accent focus:ring-2 focus:ring-accent/30"
        />
      </TextField>
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
          isDisabled={saving || draft.trim().length === 0}
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
