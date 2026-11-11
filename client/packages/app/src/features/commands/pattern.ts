/**
 * A `regex` parameter's pattern, made into a `RegExp` that matches what the server matches.
 *
 * The server reads patterns with Rust's `regex` crate and accepts a subset of its syntax
 * (`app::bot_command::pattern`). Most of that subset means the same in a `u`-mode `RegExp`;
 * `\d`, `\w`, `\s`, `\b`, their negations, and `.` do not, since Rust's follow Unicode and
 * JavaScript's are ASCII's (or, for `\s` and `.`, a different set of spaces and line ends). So
 * the pattern is read token by token and those are written as the classes Rust means by them.
 * A pattern outside the subset is not read at all: the server decides.
 * `spec/bot_command_pattern_vectors.json` holds cases both sides pass.
 */

/** What `\w` holds, as the members of a class. */
const WORD = "\\p{Alphabetic}\\p{M}\\p{Nd}\\p{Pc}\\p{Join_Control}";
const DIGIT = "\\p{Nd}";
const SPACE = "\\p{White_Space}";

/** `\b`: a word character on exactly one side. */
const BOUNDARY = `(?:(?<=[${WORD}])(?![${WORD}])|(?<![${WORD}])(?=[${WORD}]))`;
const NOT_BOUNDARY = `(?:(?<=[${WORD}])(?=[${WORD}])|(?<![${WORD}])(?![${WORD}]))`;

/** The characters a backslash may stand before as themselves; `-` joins them inside a class. */
const ESCAPABLE = "^$\\.*+?()[]{}|/";

/**
 * What an escape is: one character, a set of them (the members of a class), everything `\w`
 * does not hold, or a word boundary.
 */
type Escape =
  | { readonly kind: "character"; readonly text: string; readonly next: number }
  | { readonly kind: "set"; readonly text: string; readonly next: number }
  | { readonly kind: "notWord" | "boundary" | "notBoundary"; readonly next: number };

/** The escape whose backslash is at `at`, or `null` when the subset has none such. */
function readEscape(chars: readonly string[], at: number, inClass: boolean): Escape | null {
  const c = chars[at + 1];
  const next = at + 2;
  const rest = () => chars.slice(next, next + 80).join("");
  switch (c) {
    case undefined:
      return null;
    case "d":
      return { kind: "set", text: DIGIT, next };
    case "D":
      return { kind: "set", text: "\\P{Nd}", next };
    case "s":
      return { kind: "set", text: SPACE, next };
    case "S":
      return { kind: "set", text: "\\P{White_Space}", next };
    case "w":
      return { kind: "set", text: WORD, next };
    case "W":
      return { kind: "notWord", next };
    case "b":
      return inClass ? null : { kind: "boundary", next };
    case "B":
      return inClass ? null : { kind: "notBoundary", next };
    case "p":
    case "P": {
      const name = /^\{\w+(?:=\w+)?\}/.exec(rest())?.[0];
      return name === undefined
        ? null
        : { kind: "set", text: `\\${c}${name}`, next: next + name.length };
    }
    case "x":
    case "u": {
      const digits = (c === "x" ? /^[0-9A-Fa-f]{2}/ : /^(?:[0-9A-Fa-f]{4}|\{[0-9A-Fa-f]+\})/).exec(
        rest(),
      )?.[0];
      if (digits === undefined) {
        return null;
      }
      // Rust takes only a scalar value; JavaScript would pair two surrogates written this way.
      const code = Number.parseInt(digits.replace(/[{}]/g, ""), 16);
      return code > 0x10ffff || (code >= 0xd800 && code <= 0xdfff)
        ? null
        : { kind: "character", text: `\\${c}${digits}`, next: next + digits.length };
    }
    case "f":
    case "n":
    case "r":
    case "t":
    case "v":
      return { kind: "character", text: `\\${c}`, next };
    default:
      return ESCAPABLE.includes(c) || (inClass && c === "-")
        ? { kind: "character", text: `\\${c}`, next }
        : null;
  }
}

/** The class whose `[` is at `at`, rewritten, and where it ends; `null` outside the subset. */
function readClass(chars: readonly string[], at: number): { text: string; next: number } | null {
  let i = at + 1;
  const negated = chars[i] === "^";
  if (negated) {
    i++;
  }
  const first = i;
  let members = "";
  let notWord = false;

  /** One character at `i`, as a member or the end of a range. */
  const character = (): string | null => {
    const c = chars[i];
    if (c === undefined || c === "[" || c === "]" || c === "-") {
      return null;
    }
    if ((c === "&" || c === "~") && chars[i + 1] === c) {
      return null;
    }
    if (c === "\\") {
      const escape = readEscape(chars, i, true);
      if (escape?.kind !== "character") {
        return null;
      }
      i = escape.next;
      return escape.text;
    }
    i++;
    return c === "^" ? "\\^" : c;
  };

  for (;;) {
    const c = chars[i];
    if (c === undefined) {
      return null;
    }
    if (c === "]") {
      if (i === first) {
        return null;
      }
      i++;
      break;
    }
    if (c === "-") {
      // Unescaped, it stands first or last; between two members JavaScript reads a range.
      if (i !== first && chars[i + 1] !== "]") {
        return null;
      }
      members += "\\-";
      i++;
      continue;
    }
    if (c === "\\") {
      const escape = readEscape(chars, i, true);
      if (escape === null || escape.kind === "boundary" || escape.kind === "notBoundary") {
        return null;
      }
      if (escape.kind === "notWord") {
        notWord = true;
        i = escape.next;
        continue;
      }
      if (escape.kind === "set") {
        members += escape.text;
        i = escape.next;
        continue;
      }
    }
    const start = character();
    if (start === null) {
      return null;
    }
    members += start;
    if (chars[i] === "-" && chars[i + 1] !== "]") {
      i++;
      const end = character();
      if (end === null) {
        return null;
      }
      members += `-${end}`;
    }
  }

  // A `u`-mode class cannot hold the complement `\W` is, so it stands beside the class, the two
  // in one group for a repetition to take both.
  let text: string;
  if (!notWord) {
    text = `[${negated ? "^" : ""}${members}]`;
  } else if (negated) {
    text = members === "" ? `[${WORD}]` : `(?:(?![${members}])[${WORD}])`;
  } else {
    text = members === "" ? `[^${WORD}]` : `(?:[${members}]|[^${WORD}])`;
  }
  return { text, next: i };
}

/** `pattern` in JavaScript's terms, or `null` when it is not of the subset. */
function rewrite(pattern: string): string | null {
  const chars = Array.from(pattern);
  let out = "";
  let depth = 0;
  // Whether what was just read can be repeated: a character, a class, or a group.
  let repeatable = false;
  let i = 0;
  while (i < chars.length) {
    const c = chars[i] ?? "";
    switch (c) {
      case "\\": {
        const escape = readEscape(chars, i, false);
        if (escape === null) {
          return null;
        }
        switch (escape.kind) {
          case "boundary":
            out += BOUNDARY;
            break;
          case "notBoundary":
            out += NOT_BOUNDARY;
            break;
          case "notWord":
            out += `[^${WORD}]`;
            break;
          case "set":
            out += escape.text === WORD ? `[${WORD}]` : escape.text;
            break;
          case "character":
            out += escape.text;
        }
        repeatable = escape.kind !== "boundary" && escape.kind !== "notBoundary";
        i = escape.next;
        continue;
      }
      case "[": {
        const read = readClass(chars, i);
        if (read === null) {
          return null;
        }
        out += read.text;
        repeatable = true;
        i = read.next;
        continue;
      }
      case "(":
        if (chars[i + 1] === "?") {
          if (chars[i + 2] !== ":") {
            return null;
          }
          out += "(?:";
          i += 3;
        } else {
          out += "(";
          i++;
        }
        depth++;
        repeatable = false;
        continue;
      case ")":
        if (depth === 0) {
          return null;
        }
        depth--;
        out += ")";
        repeatable = true;
        break;
      case "|":
      case "^":
      case "$":
        out += c;
        repeatable = false;
        break;
      case "?":
      case "*":
      case "+":
      case "{": {
        if (!repeatable) {
          return null;
        }
        const count =
          c === "{" ? /^\{[0-9]+(?:,[0-9]*)?\}/.exec(chars.slice(i, i + 24).join(""))?.[0] : c;
        if (count === undefined) {
          return null;
        }
        i += count.length;
        out += count;
        if (chars[i] === "?") {
          out += "?";
          i++;
        }
        repeatable = false;
        continue;
      }
      case "]":
      case "}":
        return null;
      case ".":
        out += "[^\\n]";
        repeatable = true;
        break;
      default:
        out += c;
        repeatable = true;
    }
    i++;
  }
  return depth === 0 ? out : null;
}

/**
 * A `RegExp` matching the values `pattern` matches whole on the server, or `null` when the
 * pattern is outside the subset the server accepts or this engine cannot compile it (an older
 * one lacks look-behind, or a script the server knows).
 */
export function compilePattern(pattern: string): RegExp | null {
  const rewritten = rewrite(pattern);
  if (rewritten === null) {
    return null;
  }
  try {
    return new RegExp(`^(?:${rewritten})$`, "u");
  } catch {
    return null;
  }
}
