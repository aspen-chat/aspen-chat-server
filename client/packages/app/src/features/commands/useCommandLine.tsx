import type { Invocation, ParameterType } from "@aspen/protocol";
import { GlobeIcon, HashIcon, UsersIcon } from "@phosphor-icons/react";
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
  useChannels,
  useCommands,
  useCommunities,
  useMembers,
  useRoles,
  useStore,
  useUsers,
} from "@/api/hooks";
import { useForeignDeployments } from "@/api/deploymentsContext";
import { Avatar } from "@/features/communities/Avatar";
import { useMemberSearch } from "@/features/community-settings/memberSearch";
import { SuggestionList, type Suggestion } from "@/features/mentions/SuggestionList";
import { displayNameOf, handleOf } from "@/features/users/profile";
import { useLanguageSetting, useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import {
  commandNameAt,
  describe,
  isId,
  matchesPattern,
  messageIdIn,
  offeredCommands,
  parameterAt,
  quoteArgument,
  readCommandLine,
  sameName,
  splitArguments,
  tokenize,
  typedParameters,
  type Offered,
} from "./commandLine";

/** The most suggestions shown at once. */
const MAX_SUGGESTIONS = 8;

/** A completion: the text it puts in the box, and for a value, what that text stands for. */
interface Completion extends Suggestion {
  /** Replaces the text from `from` to the caret. */
  readonly text: string;
  readonly from: number;
  /** The value the text stands for, for an argument whose text is a name. */
  readonly value?: { type: ParameterType; id: string };
}

/** What sending the draft comes to. */
export type Prepared =
  | { readonly kind: "message" }
  | { readonly kind: "refused"; readonly reason: string }
  | { readonly kind: "command"; readonly invocation: Invocation };

/**
 * Commands in a message box. A draft beginning with `/` is a command of one of the bots that
 * can see the channel: while its name is typed, their commands are offered (name, bot, and
 * what it does, in the reader's language where the bot gave one), and picking one where
 * another bot answers to the same name writes that bot's tag after it, which is how a line
 * says which it means. While an argument is typed, the command's form is shown above the box,
 * the parameter at the caret marked, with what it takes and suggestions of that type: people,
 * roles, channels, communities, and deployments by name, which `prepare` turns into the ids
 * the server reads. `useTagging` offers nothing meanwhile (`active`).
 *
 * The keyboard and assistive technology work as `useTagging` describes.
 */
export function useCommandLine({
  channelId,
  draft,
  setDraft,
}: {
  channelId: string;
  draft: string;
  setDraft: (next: string) => void;
}) {
  const m = useMessages();
  const { resolved } = useLanguageSetting();
  const listId = useId();
  const hintId = useId();
  const store = useStore();
  const channel = useChannel(channelId);
  const parent = useChannel(channel?.parentChannel ?? "");
  const home = channel?.parentChannel != null ? parent : channel;
  const communityId = home?.community ?? "";
  const commandsNeeded = draft.startsWith("/");
  const lists = useCommands(commandsNeeded ? channelId : null);
  const offered = offeredCommands(lists ?? []);
  // Held so that the bots' names below re-render as they arrive.
  const bots = useUsers(lists?.map((l) => l.bot) ?? []);
  const usernameOf = (bot: string) => store.user(bot)?.name;
  const displayOf = (bot: string) => {
    const user = bots.find((b) => b?.id === bot);
    return user === undefined ? m.unknownUser : displayNameOf(user);
  };
  const [caret, setCaret] = useState(0);
  const [active, setActive] = useState(0);
  const [dismissedAt, setDismissedAt] = useState<string | null>(null);
  const [values, setValues] = useState<ReadonlyMap<string, { type: ParameterType; id: string }>>(
    new Map(),
  );
  const placing = useRef<{ box: HTMLTextAreaElement | null; at: number } | null>(null);

  const line = commandsNeeded ? readCommandLine(draft, offered, usernameOf) : null;
  const naming = commandNameAt(draft, caret);
  const command = line?.kind === "command" ? line : null;
  const at =
    command === null ? null : parameterAt(draft, command.argumentsAt, caret, command.command);
  const parameters = command === null ? [] : typedParameters(command.command);
  const parameter = at === null ? undefined : parameters[at.index];

  // Sources for values, read only while a parameter of their type is typed.
  const wants = (type: ParameterType) => parameter?.type === type;
  const query = (at?.token.value ?? "").replace(/^[@#]/u, "").toLowerCase();
  const sample = useMembers(wants("userId") ? communityId : "");
  const search = useMemberSearch(wants("userId") ? communityId : "", wants("userId") ? query : "");
  const recipients = useUsers(
    wants("userId") && home?.community == null ? (home?.recipients ?? []) : [],
  );
  const roles = useRoles(wants("roleId") ? communityId : "");
  const channels = useChannels(wants("channelId") ? communityId : "");
  const communities = useCommunities();
  const deployments = useForeignDeployments();

  const completions: Completion[] = [];
  let label = m.commands.suggestions;
  const key =
    naming !== null
      ? `name:${draft}`
      : at === null
        ? null
        : `${String(at.index)}:${String(at.token.start)}`;
  if (key !== null && key !== dismissedAt) {
    if (naming !== null) {
      const typed = naming.query.toLowerCase();
      for (const o of offered) {
        if (!o.command.name.toLowerCase().startsWith(typed)) {
          continue;
        }
        const shared = offered.filter((x: Offered) => sameName(x.command.name, o.command.name));
        const tag = shared.length > 1 ? ` @${usernameOf(o.bot) ?? o.bot}` : "";
        completions.push({
          key: `${o.bot}:${o.command.name}`,
          text: `/${o.command.name}${tag} `,
          from: 0,
          label: `/${o.command.name}`,
          detail: displayOf(o.bot),
          description: describe(o.command, resolved.locale),
          icon: <Avatar name={displayOf(o.bot)} iconId={store.user(o.bot)?.icon} size="sm" />,
        });
      }
    } else if (at !== null && parameter !== undefined) {
      label = format(m.commands.values, { parameter: parameter.name });
      const from = at.token.start;
      const offer = (
        id: string,
        type: ParameterType,
        shown: string,
        name: string,
        detail: string | null,
        icon: Completion["icon"],
      ) => {
        if (name.toLowerCase().includes(query) || shown.toLowerCase().includes(query)) {
          completions.push({
            key: `${type}:${id}`,
            text: `${quoteArgument(shown)} `,
            from,
            label: name,
            detail,
            icon,
            value: { type, id },
          });
        }
      };
      switch (parameter.type) {
        case "userId": {
          const people =
            home?.community == null
              ? recipients.flatMap((u) => (u === undefined ? [] : [u]))
              : [
                  ...search.members,
                  ...sample.filter((u) => !search.members.some((f) => f.id === u.id)),
                ];
          for (const user of people) {
            offer(
              user.id,
              "userId",
              `@${user.name}`,
              displayNameOf(user),
              handleOf(user),
              <Avatar name={displayNameOf(user)} iconId={user.icon} size="sm" />,
            );
          }
          break;
        }
        case "roleId":
          for (const role of roles) {
            offer(
              role.id,
              "roleId",
              `@${role.name}`,
              `@${role.name}`,
              null,
              <UsersIcon size={18} aria-hidden="true" className="text-ink-muted" />,
            );
          }
          break;
        case "channelId":
          for (const c of channels) {
            offer(
              c.id,
              "channelId",
              `#${c.name}`,
              `#${c.name}`,
              null,
              <HashIcon size={18} aria-hidden="true" className="text-ink-muted" />,
            );
          }
          break;
        case "communityId":
          for (const c of communities) {
            offer(
              c.id,
              "communityId",
              c.name,
              c.name,
              null,
              <Avatar name={c.name} iconId={c.icon} size="sm" />,
            );
          }
          break;
        case "deploymentHost":
          for (const d of deployments) {
            offer(
              d.domain,
              "deploymentHost",
              d.domain,
              d.domain,
              null,
              <GlobeIcon size={18} aria-hidden="true" className="text-ink-muted" />,
            );
          }
          break;
        default:
          break;
      }
    }
  }
  const shown = completions.slice(0, MAX_SUGGESTIONS);
  const current = Math.min(active, shown.length - 1);
  const hinted =
    naming === null && command !== null && parameter !== undefined
      ? { command: command.command, parameter }
      : null;
  const open = shown.length > 0;

  const pick = useCallback(
    (completion: Completion, box: HTMLTextAreaElement | null) => {
      const before = draft.slice(0, completion.from) + completion.text;
      setDraft(before + draft.slice(caret).trimStart());
      if (completion.value !== undefined) {
        const shownText = completion.text.trim();
        const value = completion.value;
        setValues((held) => new Map(held).set(unquote(shownText), value));
      }
      setActive(0);
      setCaret(before.length);
      placing.current = { box, at: before.length };
    },
    [draft, caret, setDraft, setValues],
  );

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
      setDismissedAt(key);
      return true;
    }
    return false;
  };

  const follow = (event: SyntheticEvent<HTMLTextAreaElement>) => {
    setCaret(event.currentTarget.selectionStart);
  };

  const fits =
    parameter?.type !== "regex" ||
    at === null ||
    at.token.value === "" ||
    matchesPattern(parameter.pattern ?? "", at.token.value);

  const hint =
    hinted !== null ? (
      <div id={hintId} className="flex flex-col gap-0.5 px-2 py-1.5 text-sm">
        <p className="font-mono text-xs text-ink-muted" dir="auto">
          <span className="text-ink">/{hinted.command.name}</span>
          {(hinted.command.parameters ?? []).map((p, index) => {
            const marked = p === hinted.parameter;
            const text = p.optional === true ? `[${p.name}]` : `<${p.name}>`;
            return (
              <span key={`${p.name}:${String(index)}`}>
                {" "}
                {marked ? (
                  <mark className="rounded bg-accent-soft px-0.5 text-accent-strong">{text}</mark>
                ) : (
                  text
                )}
              </span>
            );
          })}
        </p>
        <p>
          <span className="font-medium">{hinted.parameter.name}</span>
          {hinted.parameter.optional === true && (
            <span className="text-ink-muted"> ({m.commands.optional})</span>
          )}
          {" · "}
          <span className="text-ink-muted">{m.commands.takes[hinted.parameter.type]}</span>
        </p>
        <p className="text-ink-muted">{describe(hinted.parameter, resolved.locale)}</p>
        {!fits && (
          <p className="text-danger">
            {format(m.commands.doesNotFit, { parameter: hinted.parameter.name })}
          </p>
        )}
      </div>
    ) : undefined;

  const shownList =
    open || hint !== undefined ? (
      <SuggestionList
        id={listId}
        label={label}
        suggestions={shown}
        current={current}
        onPick={pick}
        header={hint}
      />
    ) : null;

  const announcement = !open
    ? ""
    : naming !== null
      ? format(shown.length === 1 ? m.commands.oneSuggestion : m.commands.someSuggestions, {
          count: String(shown.length),
        })
      : format(shown.length === 1 ? m.commands.oneValue : m.commands.someValues, {
          count: String(shown.length),
          parameter: parameter?.name ?? "",
        });

  /**
   * What sending the draft comes to: a message where it is not a command of a bot here, a
   * command with its arguments as ids where it is, or why it cannot be sent as it stands.
   * `attachments` are the files going with it, which its file parameters take in order.
   */
  const prepare = (text: string, attachments: readonly string[]): Prepared => {
    const read = readCommandLine(
      text,
      offeredCommands(store.commands(channelId) ?? []),
      usernameOf,
    );
    if (read === null || read.kind === "unknown") {
      return { kind: "message" };
    }
    if (read.kind === "ambiguous") {
      const names = read.bots.map((bot) => `@${usernameOf(bot) ?? bot}`);
      return {
        kind: "refused",
        reason: format(m.commands.ambiguous, {
          name: read.name,
          bots: new Intl.ListFormat(resolved.locale, { type: "conjunction" }).format(names),
          example: usernameOf(read.bots[0] ?? "") ?? "",
        }),
      };
    }
    const split = splitArguments(text, read.argumentsAt, read.command);
    // More arguments than parameters go as they are, for the server to say how many it takes.
    const typed =
      split.kind === "ok"
        ? [...split.values]
        : tokenize(text, read.argumentsAt).map((t) => t.value);
    const files = [...attachments];
    const args: string[] = [];
    for (const p of read.command.parameters ?? []) {
      if (p.type === "attachmentId") {
        const file = files.shift();
        if (file === undefined) {
          break;
        }
        args.push(file);
        continue;
      }
      const value = typed.shift();
      if (value === undefined) {
        break;
      }
      args.push(resolve(p.type, value));
    }
    args.push(...typed);
    return {
      kind: "command",
      invocation: {
        bot: read.bot,
        name: read.command.name,
        arguments: args,
        attachments: [...attachments],
      },
    };
  };

  /** A typed value as the server reads it: a name picked or known as its id. */
  const resolve = (type: ParameterType, value: string): string => {
    const picked = values.get(value);
    if (picked?.type === type) {
      return picked.id;
    }
    const trimmed = value.trim();
    switch (type) {
      case "userId": {
        const tag = /^<@([0-9a-fA-F-]{36})>$/u.exec(trimmed);
        if (tag?.[1] !== undefined) {
          return tag[1];
        }
        if (trimmed.startsWith("@")) {
          const user = store.userNamed(trimmed.slice(1));
          if (user !== undefined) {
            return user.id;
          }
        }
        return trimmed;
      }
      case "roleId": {
        const tag = /^<@&([0-9a-fA-F-]{36})>$/u.exec(trimmed);
        if (tag?.[1] !== undefined) {
          return tag[1];
        }
        const role = store
          .roles(communityId)
          .find((r) => sameName(`@${r.name}`, trimmed) || sameName(r.name, trimmed));
        return role?.id ?? trimmed;
      }
      case "channelId": {
        if (isId(trimmed)) {
          return trimmed;
        }
        const found = store
          .channels(communityId)
          .find((c) => sameName(`#${c.name}`, trimmed) || sameName(c.name, trimmed));
        return found?.id ?? trimmed;
      }
      case "communityId": {
        const found = store.communities().find((c) => sameName(c.name, trimmed));
        return found?.id ?? trimmed;
      }
      case "messageId":
        return messageIdIn(trimmed) ?? trimmed;
      default:
        return value;
    }
  };

  return {
    /** Whether the draft is a command line, so tagging offers nothing. */
    active: commandsNeeded,
    follow,
    /** What the box carries while the list is open. */
    aria: {
      "aria-autocomplete": "list" as const,
      "aria-controls": open ? listId : undefined,
      "aria-activedescendant": open ? `${listId}-${String(current)}` : undefined,
      "aria-describedby": hint !== undefined ? hintId : undefined,
    },
    onKeyDown,
    list: shownList,
    /** What a polite status beside the box says of the suggestions: how many, how to pick. */
    announcement,
    prepare,
    /** Forgets the values picked, once what they were for is sent. */
    reset: () => {
      setValues(new Map());
      setDismissedAt(null);
    },
  };
}

function unquote(text: string): string {
  return text.startsWith('"') && text.endsWith('"') && text.length >= 2
    ? text.slice(1, -1).replaceAll('\\"', '"').replaceAll("\\\\", "\\")
    : text;
}
