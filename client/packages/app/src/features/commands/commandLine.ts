import type { BotCommands, Command, CommandParameter } from "@aspen/protocol";

/**
 * Reading a command as a person types it in the message box: `/name`, then optionally a tag
 * of the bot meant where several answer to the name, then the arguments in order. An argument
 * holding whitespace or a quote is quoted (`"for luck"`, with `\"` and `\\` inside), as the
 * server writes it back; the last parameter, when it takes any text, takes the rest of the line
 * as written. Files are not typed: an `attachmentId` parameter takes the files attached to the
 * message, in order.
 */

/** One command a bot in the channel answers. */
export interface Offered {
  readonly bot: string;
  readonly command: Command;
}

/** Every bot's commands, one entry per command. */
export function offeredCommands(lists: readonly BotCommands[]): Offered[] {
  return lists.flatMap((list) => list.commands.map((command) => ({ bot: list.bot, command })));
}

/** Names compare ignoring case, as the server keeps them unique. */
export function sameName(a: string, b: string): boolean {
  return a.localeCompare(b, undefined, { sensitivity: "accent" }) === 0;
}

/** The command's name being typed: the draft's first word, while the caret is still in it. */
export function commandNameAt(draft: string, caret: number): { query: string } | null {
  const match = /^\/(\S*)$/u.exec(draft.slice(0, caret));
  return match === null ? null : { query: match[1] ?? "" };
}

/** A word of a command line: what it says, and where it stands in the text. */
export interface Token {
  readonly value: string;
  readonly start: number;
  readonly end: number;
}

/**
 * The words of `text` from `from` on, split at whitespace outside quotes. A quote left open
 * runs to the end, so a word being typed reads as one.
 */
export function tokenize(text: string, from = 0): Token[] {
  const tokens: Token[] = [];
  let i = from;
  while (i < text.length) {
    while (i < text.length && /\s/u.test(text[i] ?? "")) {
      i++;
    }
    if (i >= text.length) {
      break;
    }
    const start = i;
    let value = "";
    if (text[i] === '"') {
      i++;
      while (i < text.length && text[i] !== '"') {
        const c = text[i] ?? "";
        const next = text[i + 1];
        if (c === "\\" && (next === '"' || next === "\\")) {
          value += next;
          i += 2;
        } else {
          value += c;
          i++;
        }
      }
      i = Math.min(i + 1, text.length);
    } else {
      while (i < text.length && !/\s/u.test(text[i] ?? "")) {
        value += text[i] ?? "";
        i++;
      }
    }
    tokens.push({ value, start, end: i });
  }
  return tokens;
}

/** A value as a command line writes it: quoted where it holds whitespace or a quote. */
export function quoteArgument(value: string): string {
  return value !== "" && !/[\s"]/u.test(value)
    ? value
    : `"${value.replaceAll("\\", "\\\\").replaceAll('"', '\\"')}"`;
}

/** The parameters typed on the line, which is every one but those files fill. */
export function typedParameters(command: Command): CommandParameter[] {
  return (command.parameters ?? []).filter((p) => p.type !== "attachmentId");
}

/** What a draft beginning with `/` asks for. */
export type CommandLine =
  | { readonly kind: "unknown"; readonly name: string }
  | { readonly kind: "ambiguous"; readonly name: string; readonly bots: readonly string[] }
  | {
      readonly kind: "command";
      readonly bot: string;
      readonly command: Command;
      readonly argumentsAt: number;
    };

/**
 * The command a draft names, or `null` when it does not begin with `/`. `usernameOf` gives a
 * bot's username, which a tag after the name (`@name`, or `<@id>`) is matched against.
 */
export function readCommandLine(
  draft: string,
  offered: readonly Offered[],
  usernameOf: (bot: string) => string | undefined,
): CommandLine | null {
  if (!draft.startsWith("/")) {
    return null;
  }
  const [first, second] = tokenize(draft, 1);
  if (first?.start !== 1) {
    return { kind: "unknown", name: "" };
  }
  const name = first.value;
  const answering = offered.filter((o) => sameName(o.command.name, name));
  if (answering.length === 0) {
    return { kind: "unknown", name };
  }
  const tag = second?.value ?? "";
  const tagged = answering.find(
    (o) =>
      tag === `<@${o.bot}>` ||
      (tag.startsWith("@") && sameName(usernameOf(o.bot) ?? "", tag.slice(1))),
  );
  if (tagged !== undefined && second !== undefined) {
    return { kind: "command", bot: tagged.bot, command: tagged.command, argumentsAt: second.end };
  }
  const [only, ...others] = answering;
  if (only !== undefined && others.length === 0) {
    return { kind: "command", bot: only.bot, command: only.command, argumentsAt: first.end };
  }
  return { kind: "ambiguous", name, bots: answering.map((o) => o.bot) };
}

/** The typed arguments, one per typed parameter given, or why they do not fit. */
export type SplitArguments =
  { readonly kind: "ok"; readonly values: readonly string[] } | { readonly kind: "tooMany" };

/**
 * The arguments written after a command, one per typed parameter in order. The last, when it
 * takes any text, takes the rest of the line as written, unquoted only when that is a single
 * quoted word.
 */
export function splitArguments(text: string, from: number, command: Command): SplitArguments {
  const parameters = typedParameters(command);
  const tokens = tokenize(text, from);
  const last = parameters.at(-1);
  if (last?.type === "any" && tokens.length >= parameters.length) {
    const rest = tokens.slice(parameters.length - 1);
    const head = tokens.slice(0, parameters.length - 1).map((t) => t.value);
    const first = rest[0];
    const whole =
      rest.length === 1 && first !== undefined
        ? first.value
        : text.slice(first?.start ?? text.length).trim();
    return { kind: "ok", values: [...head, whole] };
  }
  if (tokens.length > parameters.length) {
    return { kind: "tooMany" };
  }
  return { kind: "ok", values: tokens.map((t) => t.value) };
}

/**
 * Which typed parameter the caret is in, and the word it is typing (empty between words), for
 * hints and suggestions. `null` past the last parameter, or while the caret is before the
 * arguments.
 */
export function parameterAt(
  text: string,
  from: number,
  caret: number,
  command: Command,
): { index: number; token: Token } | null {
  if (caret < from) {
    return null;
  }
  const parameters = typedParameters(command);
  const tokens = tokenize(text.slice(0, caret), from);
  const lastToken = tokens.at(-1);
  const inWord = lastToken?.end === caret && caret > from;
  const index = inWord ? tokens.length - 1 : tokens.length;
  const token = inWord ? lastToken : { value: "", start: caret, end: caret };
  const final = parameters.length - 1;
  if (index > final) {
    // Everything past the last parameter is still its value when it takes any text.
    return parameters[final]?.type === "any" ? { index: final, token } : null;
  }
  return { index, token };
}

/** A message's link or bare id, as the message it names. */
export function messageIdIn(value: string): string | null {
  const match =
    /(?:^|\/messages\/)([0-9a-fA-F]{8}-(?:[0-9a-fA-F]{4}-){3}[0-9a-fA-F]{12})(?:[/?#]|$)/u.exec(
      value.trim(),
    );
  return match?.[1]?.toLowerCase() ?? null;
}

/** A bare id, as ids are written. */
export function isId(value: string): boolean {
  return /^[0-9a-fA-F]{8}-(?:[0-9a-fA-F]{4}-){3}[0-9a-fA-F]{12}$/u.test(value.trim());
}

/**
 * Whether `value` matches a `regex` parameter's pattern whole, as the server will check it; a
 * pattern this engine cannot read is left to the server.
 */
export function matchesPattern(pattern: string, value: string): boolean {
  try {
    return new RegExp(`^(?:${pattern})$`, "u").test(value);
  } catch {
    return true;
  }
}

/**
 * A description in the reader's language where the bot gave one: the locale whole, then its
 * language alone, then the bot's own.
 */
export function describe(
  item: { description: string; descriptions?: Record<string, string> },
  locale: string,
): string {
  const descriptions = item.descriptions ?? {};
  const entries = Object.entries(descriptions);
  const exact = entries.find(([tag]) => tag.toLowerCase() === locale.toLowerCase());
  if (exact !== undefined) {
    return exact[1];
  }
  const language = locale.split("-")[0]?.toLowerCase() ?? "";
  const loose = entries.find(([tag]) => tag.split("-")[0]?.toLowerCase() === language);
  return loose?.[1] ?? item.description;
}
