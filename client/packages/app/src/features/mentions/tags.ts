/**
 * Tags in the text a person writes. The message box shows a picked tag as `@username` (unique,
 * so it reads back unambiguously) or `@Role name`, and remembers what each stands for; sending
 * turns them into the `<@user-id>` and `<@&role-id>` the server reads. `@everyone` is written
 * as it is sent.
 */

/** A tag picked in the message box: the text it shows as, and what it is sent as. */
export interface PickedTag {
  readonly text: string;
  readonly token: string;
}

/** The `@` word being typed just before `caret`, if any: where it starts and what follows it. */
export function tagQueryAt(text: string, caret: number): { start: number; query: string } | null {
  const match = /(^|[\s([{])@([^\s@<>]{0,32})$/u.exec(text.slice(0, caret));
  if (match === null) {
    return null;
  }
  return { start: match.index + (match[1]?.length ?? 0), query: match[2] ?? "" };
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** `text` with each picked tag's shown form, where it stands as a whole word, as sent. */
export function encodeTags(text: string, picks: readonly PickedTag[]): string {
  let result = text;
  // Longer forms first, so `@Team leads` is not taken for `@Team`.
  for (const pick of [...picks].sort((a, b) => b.text.length - a.text.length)) {
    result = result.replace(
      new RegExp(`${escapeRegExp(pick.text)}(?![\\p{L}\\p{N}_])`, "gu"),
      pick.token,
    );
  }
  return result;
}

/**
 * A sent message's text as the message box shows it, and the tags it holds, so an edit reads
 * as it was written and is sent back the same way. A tag naming someone or something unknown
 * is left as it was sent.
 */
export function decodeTags(
  content: string,
  usernameOf: (id: string) => string | undefined,
  roleNameOf: (id: string) => string | undefined,
): { text: string; picks: PickedTag[] } {
  const picks: PickedTag[] = [];
  const text = content.replace(/<@(&?)([0-9a-fA-F-]{36})>/g, (token, role: string, id: string) => {
    const name = role === "&" ? roleNameOf(id) : usernameOf(id);
    if (name === undefined) {
      return token;
    }
    const shown = `@${name}`;
    if (!picks.some((pick) => pick.token === token)) {
      picks.push({ text: shown, token });
    }
    return shown;
  });
  return { text, picks };
}
