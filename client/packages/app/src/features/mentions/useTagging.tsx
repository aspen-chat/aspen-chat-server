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
import { tagQueryAt } from "@/features/mentions/tags";
import type { PickTarget, Splice } from "@/features/messages/picks";
import { displayNameOf, handleOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const MAX_SUGGESTIONS = 8;

interface Tag extends Suggestion {
  /** What the message box shows once picked. */
  readonly text: string;
  /** What the text stands for; none for what is sent as it is written (`@everyone`). */
  readonly target?: PickTarget;
}

/**
 * Tagging in a message box. Typing `@` offers the people of the channel's community or DM, its
 * roles, and `@everyone`, each only with the permission
 * to tag it in this channel; the arrow keys choose, Enter or Tab picks, and Escape dismisses.
 * A picked tag shows as `@username` or `@Role` and is written through `splice`, which remembers
 * where it stands and what it tags (`picks.ts`).
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
  splice,
  off = false,
}: {
  channelId: string;
  draft: string;
  /** Writes into the draft, remembering a pick (`usePickedText`). */
  splice: Splice;
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
            target: { kind: "user", id: user.id },
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
            target: { kind: "role", id: role.id },
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
      splice(typing.start, caret, suggestion.text, suggestion.target, " ");
      const after = typing.start + suggestion.text.length + 1;
      setActive(0);
      setCaret(after);
      placing.current = { box, at: after };
    },
    [typing, caret, splice],
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

  /**
   * Puts a tag of `target`, shown as `text`, at the end of the draft, picked as though chosen
   * from the list, from somewhere outside the box (a user's Mention); the caret goes after it,
   * in `box` when given.
   */
  const insert = useCallback(
    (text: string, target: PickTarget, box: HTMLTextAreaElement | null) => {
      let at = draft.length;
      if (draft !== "" && !/\s$/u.test(draft)) {
        splice(at, at, " ");
        at += 1;
      }
      splice(at, at, text, target, " ");
      const after = at + text.length + 1;
      setCaret(after);
      placing.current = { box, at: after };
    },
    [draft, splice],
  );

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
    ? format(m.tagging.suggestionsAnnouncement, { count: shown.length })
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
    insert,
    /** Forgets where the list was dismissed, once the box holds other text. */
    reset: () => {
      setDismissedAt(null);
    },
  };
}
