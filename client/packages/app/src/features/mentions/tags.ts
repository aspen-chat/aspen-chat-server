/**
 * Tags in the text a person writes. The message box shows a picked tag as `@username` or
 * `@Role name` and remembers it by its place in the text (`picks.ts`), since neither name is
 * one target's alone; sending turns each into the `<@user-id>` or `<@&role-id>` the server
 * reads. `@everyone` is written as it is sent.
 */

/** A tag as the server reads it: whether it is a role's (`&`), and the id. */
export const TAG_REFERENCE = "<@(&?)([0-9a-fA-F-]{36})>";

/** The `@` word being typed just before `caret`, if any: where it starts and what follows it. */
export function tagQueryAt(text: string, caret: number): { start: number; query: string } | null {
  const match = /(^|[\s([{])@([^\s@<>]{0,32})$/u.exec(text.slice(0, caret));
  if (match === null) {
    return null;
  }
  return { start: match.index + (match[1]?.length ?? 0), query: match[2] ?? "" };
}

/**
 * A sent message's text with its tags read as names, `@username` and `@Role name`, for showing
 * a line of it. A tag naming someone or something unknown is left as it was sent.
 */
export function decodeTags(
  content: string,
  usernameOf: (id: string) => string | undefined,
  roleNameOf: (id: string) => string | undefined,
): string {
  return content.replace(new RegExp(TAG_REFERENCE, "g"), (token, role: string, id: string) => {
    const name = role === "&" ? roleNameOf(id) : usernameOf(id);
    return name === undefined ? token : `@${name}`;
  });
}
