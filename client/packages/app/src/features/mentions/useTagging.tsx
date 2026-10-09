import type { User } from "@aspen/protocol";
import { UsersIcon, UsersThreeIcon } from "@phosphor-icons/react";
import {
  useCallback,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type SyntheticEvent,
} from "react";
import {
  useChannel,
  useChannelAccess,
  useMe,
  useMembers,
  useNicknames,
  useRoles,
  useUsers,
} from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { useMemberSearch } from "@/features/community-settings/memberSearch";
import { SuggestionList, type Suggestion } from "@/features/mentions/SuggestionList";
import { encodeTags, tagQueryAt, type PickedTag } from "@/features/mentions/tags";
import { displayNameOf, handleOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const MAX_SUGGESTIONS = 8;

interface Tag extends Suggestion {
  /** What the message box shows once picked. */
  readonly text: string;
  /** What it is sent as. */
  readonly token: string;
}

/**
 * Tagging in a message box. Typing `@` offers the people of the channel's community or DM, its
 * roles, and `@everyone`, each only with the permission
 * to tag it in this channel; the arrow keys choose, Enter or Tab picks, and Escape dismisses.
 * A picked tag shows as `@username` or `@Role`, and `encode` turns the text into what is sent.
 * People come from the member sample, joined as the caller types by a search of every member
 * where the server lets them search (`useMemberSearch`), so a large community's members can be
 * tagged by name though the sample holds only the most recently seen.
 *
 * React Aria's ComboBox needs an `Input` of its own and completes the whole field, so it
 * cannot offer completions at the caret of a multi-line `TextArea`; this follows its pattern
 * instead, with what a text box may carry: `aria-autocomplete`, the list named by
 * `aria-controls`, and the chosen option by `aria-activedescendant`, focus staying in the box.
 * `announcement` is what a polite status beside the box says when suggestions appear and how
 * to pick one, since a text box has no `aria-expanded` to announce it; the box renders the one
 * status, which other completions (`useCommandLine`) share.
 */
export function useTagging({
  channelId,
  draft,
  setDraft,
  initialPicks = [],
  off = false,
}: {
  channelId: string;
  draft: string;
  setDraft: (next: string) => void;
  /** The tags a message being edited already holds (`decodeTags`). */
  initialPicks?: readonly PickedTag[];
  /** Offers nothing, while something else completes what is typed (a command's arguments). */
  off?: boolean;
}) {
  const m = useMessages();
  const listId = useId();
  const me = useMe();
  const channel = useChannel(channelId);
  const parent = useChannel(channel?.parentChannel ?? "");
  const home = channel?.parentChannel != null ? parent : channel;
  const access = useChannelAccess(channelId);
  const sample = useMembers(home?.community ?? "");
  const recipients = useUsers(home?.community == null ? (home?.recipients ?? []) : []);
  const roles = useRoles(home?.community ?? "");
  const nicknames = useNicknames(home?.community);
  const [caret, setCaret] = useState(0);
  const [picks, setPicks] = useState<readonly PickedTag[]>(initialPicks);
  const [active, setActive] = useState(0);
  const [dismissedAt, setDismissedAt] = useState<number | null>(null);
  /** Where to put the caret once a pick's text is committed. */
  const placing = useRef<{ box: HTMLTextAreaElement | null; at: number } | null>(null);

  const typing = tagQueryAt(draft, caret);
  const query = typing?.query.toLowerCase() ?? "";
  const search = useMemberSearch(home?.community ?? "", typing === null ? "" : query);
  const people: readonly User[] =
    home?.community == null
      ? recipients.filter((u): u is User => u !== undefined)
      : [...search.members, ...sample.filter((u) => !search.members.some((f) => f.id === u.id))];
  const suggestions: Tag[] = [];
  if (!off && typing !== null && typing.start !== dismissedAt) {
    if (access.has("mentionMembers")) {
      for (const user of people) {
        const nickname = nicknames.get(user.id);
        if (
          user.id !== me?.id &&
          (user.name.toLowerCase().includes(query) ||
            displayNameOf(user).toLowerCase().includes(query) ||
            (nickname?.toLowerCase().includes(query) ?? false))
        ) {
          const called = nickname ?? displayNameOf(user);
          suggestions.push({
            key: `user:${user.id}`,
            text: `@${user.name}`,
            token: `<@${user.id}>`,
            label: called,
            detail: handleOf(user),
            icon: <Avatar name={called} iconId={user.icon} size="sm" />,
          });
        }
      }
    }
    if (access.has("mentionRoles")) {
      for (const role of roles) {
        if (!role.everyone && role.name.toLowerCase().includes(query)) {
          suggestions.push({
            key: `role:${role.id}`,
            text: `@${role.name}`,
            token: `<@&${role.id}>`,
            label: `@${role.name}`,
            detail: m.tagging.role,
            icon: <UsersIcon size={18} aria-hidden="true" className="text-ink-muted" />,
          });
        }
      }
    }
    if (access.has("mentionEveryone") && "everyone".startsWith(query)) {
      suggestions.push({
        key: "everyone",
        text: "@everyone",
        token: "@everyone",
        label: "@everyone",
        detail: m.tagging.everyone,
        icon: <UsersThreeIcon size={18} aria-hidden="true" className="text-ink-muted" />,
      });
    }
  }
  const shown = suggestions.slice(0, MAX_SUGGESTIONS);
  const open = shown.length > 0;
  const current = Math.min(active, shown.length - 1);

  const pick = useCallback(
    (suggestion: Tag, box: HTMLTextAreaElement | null) => {
      if (typing === null) {
        return;
      }
      const before = draft.slice(0, typing.start) + suggestion.text + " ";
      setDraft(before + draft.slice(caret));
      setPicks((held) =>
        held.some((p) => p.token === suggestion.token)
          ? held
          : [...held, { text: suggestion.text, token: suggestion.token }],
      );
      setActive(0);
      setCaret(before.length);
      placing.current = { box, at: before.length };
    },
    [typing, draft, caret, setDraft],
  );

  // The caret goes after the picked tag as the new text is committed, before anything else
  // typed can land, which a later frame would not promise.
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

  const boxProps = {
    onSelect: followCaret,
    onKeyUp: followCaret,
    onClick: followCaret,
    "aria-autocomplete": "list" as const,
    "aria-controls": open ? listId : undefined,
    "aria-activedescendant": open ? `${listId}-${String(current)}` : undefined,
  };

  const announcement = open
    ? format(shown.length === 1 ? m.tagging.oneSuggestion : m.tagging.someSuggestions, {
        count: String(shown.length),
      })
    : "";

  const suggestionList = open ? (
    <SuggestionList
      id={listId}
      label={m.tagging.suggestions}
      suggestions={shown}
      current={current}
      onPick={pick}
    />
  ) : null;

  return {
    boxProps,
    onKeyDown,
    list: suggestionList,
    /** What a polite status beside the box says of the suggestions: how many, how to pick. */
    announcement,
    /** The tags picked, which a draft keeps with its text. */
    picks,
    /** The text as it is sent, picked tags and all. */
    encode: (text: string) => encodeTags(text, picks),
    /** Forgets the picks, once what they were for is sent. */
    reset: () => {
      setPicks([]);
      setDismissedAt(null);
    },
  };
}
