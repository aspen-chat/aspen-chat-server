import type { CustomEmoji } from "@aspen/protocol";
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type SyntheticEvent,
} from "react";
import { useChannel, useCustomEmoji } from "@/api/hooks";
import { CustomEmojiGlyph } from "@/features/emoji/CustomEmojiGlyph";
import { encodeCustomEmoji } from "@/features/emoji/customEmoji";
import { loadEmojiNames, searchEmojiNames, type NamedEmoji } from "@/features/emoji/emojiNames";
import { SuggestionList, type Suggestion } from "@/features/mentions/SuggestionList";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const MAX_SUGGESTIONS = 8;
/** How much of a name must be typed after the colon before anything is offered. */
const MIN_QUERY_CHARS = 2;

/** An emoji offered for what is typed: what the pick writes into the box. */
interface EmojiSuggestion extends Suggestion {
  readonly text: string;
}

/**
 * The `:` word being typed just before `caret`, if any: where it starts and what follows the
 * colon. A colon inside a word (a time, a URL) begins nothing.
 */
export function emojiQueryAt(text: string, caret: number): { start: number; query: string } | null {
  const match = /(^|[\s([{]):([^\s:]{0,32})$/u.exec(text.slice(0, caret));
  if (match === null) {
    return null;
  }
  return { start: match.index + (match[1]?.length ?? 0), query: match[2] ?? "" };
}

/**
 * Completes emoji in a message box: a colon followed by part of a name offers the community's
 * own emoji and every unicode emoji that answers to it, the community's first. A unicode pick
 * writes the glyph; a custom pick writes `:name:`, which `encode` turns into the emoji's
 * reference as the message is sent. The list is keyed as `useTagging`'s is, and reads the
 * same way.
 */
export function useEmojiCompletion({
  channelId,
  draft,
  setDraft,
  off = false,
}: {
  channelId: string;
  draft: string;
  setDraft: (next: string) => void;
  /** Offers nothing, while something else completes what is typed. */
  off?: boolean;
}) {
  const m = useMessages();
  const listId = useId();
  const channel = useChannel(channelId);
  const parent = useChannel(channel?.parentChannel ?? "");
  const home = channel?.parentChannel != null ? parent : channel;
  const communityId = home?.community ?? null;
  const custom = useCustomEmoji(communityId ?? "");
  const [named, setNamed] = useState<readonly NamedEmoji[] | null>(null);
  const [caret, setCaret] = useState(0);
  const [active, setActive] = useState(0);
  const [dismissedAt, setDismissedAt] = useState<number | null>(null);
  const placing = useRef<{ box: HTMLTextAreaElement | null; at: number } | null>(null);

  const typing = off ? null : emojiQueryAt(draft, caret);
  const query = typing?.query ?? "";
  const asking = typing !== null && query.length >= MIN_QUERY_CHARS && typing.start !== dismissedAt;

  // The names load the first time they are asked for, and are kept.
  useEffect(() => {
    if (asking && named === null) {
      let live = true;
      void loadEmojiNames().then((loaded) => {
        if (live) {
          setNamed(loaded);
        }
      });
      return () => {
        live = false;
      };
    }
    return undefined;
  }, [asking, named]);

  const suggestions: EmojiSuggestion[] = [];
  if (asking) {
    const q = query.toLowerCase();
    for (const emoji of custom) {
      if (emoji.name.toLowerCase().includes(q)) {
        suggestions.push({
          key: `custom:${emoji.id}`,
          text: `:${emoji.name}:`,
          label: `:${emoji.name}:`,
          detail: m.emoji.customDetail,
          icon: <CustomEmojiGlyph id={emoji.id} communityId={communityId} size="large" />,
        });
      }
    }
    if (named !== null && suggestions.length < MAX_SUGGESTIONS) {
      for (const emoji of searchEmojiNames(named, q, MAX_SUGGESTIONS - suggestions.length)) {
        suggestions.push({
          key: `unicode:${emoji.glyph}`,
          text: emoji.glyph,
          label: `:${emoji.names[emoji.names.length - 1] ?? ""}:`,
          detail: null,
          icon: (
            <span className="text-[1.5em] leading-none" aria-hidden="true">
              {emoji.glyph}
            </span>
          ),
        });
      }
    }
  }
  const shown = suggestions.slice(0, MAX_SUGGESTIONS);
  const open = shown.length > 0;
  const current = Math.min(active, shown.length - 1);

  const pick = useCallback(
    (suggestion: EmojiSuggestion, box: HTMLTextAreaElement | null) => {
      if (typing === null) {
        return;
      }
      const before = draft.slice(0, typing.start) + suggestion.text + " ";
      setDraft(before + draft.slice(caret));
      setActive(0);
      setCaret(before.length);
      placing.current = { box, at: before.length };
    },
    [typing, draft, caret, setDraft],
  );

  // The caret goes after the pick as the new text is committed, before anything else typed
  // can land.
  useLayoutEffect(() => {
    const pending = placing.current;
    if (pending !== null) {
      placing.current = null;
      pending.box?.setSelectionRange(pending.at, pending.at);
    }
  });

  /** Keys the list takes while it is open; returns whether it took this one. */
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>): boolean => {
    if (!open) {
      return false;
    }
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      const step = event.key === "ArrowDown" ? 1 : -1;
      setActive((current + step + shown.length) % shown.length);
      return true;
    }
    if (event.key === "Enter" || event.key === "Tab") {
      const chosen = shown[current];
      if (chosen !== undefined) {
        event.preventDefault();
        pick(chosen, event.currentTarget);
        return true;
      }
    }
    if (event.key === "Escape") {
      event.preventDefault();
      setDismissedAt(typing?.start ?? null);
      return true;
    }
    return false;
  };

  const followCaret = (event: SyntheticEvent<HTMLTextAreaElement>) => {
    setCaret(event.currentTarget.selectionStart);
  };

  const announcement = open
    ? format(shown.length === 1 ? m.emoji.oneSuggestion : m.emoji.someSuggestions, {
        count: String(shown.length),
      })
    : "";

  return {
    /** Whether the list is open, which decides which completion names the box's list. */
    open,
    listId,
    current,
    follow: followCaret,
    onKeyDown,
    list: open ? (
      <SuggestionList
        id={listId}
        label={m.emoji.suggestions}
        suggestions={shown}
        current={current}
        onPick={pick}
      />
    ) : null,
    announcement,
    /** The text as it is sent: each `:name:` of the community's emoji as its reference. */
    encode: (text: string) => encodeCustomEmoji(text, custom),
    /** The community's emoji, for decoding a message being edited. */
    custom,
  };
}

export type { CustomEmoji };
