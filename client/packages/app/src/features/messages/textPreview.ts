/** How many lines of a text or code attachment show in its message. */
export const TEXT_PREVIEW_LINES = 20;

/**
 * The most of a text or code attachment read for its preview, in bytes: twenty long lines, and
 * a bound on what a file of one enormous line costs the reader. Reading stops there.
 */
export const TEXT_PREVIEW_MAX_BYTES = 32 * 1024;

/** Text types outside `text/` that are read as text. */
const TEXT_TYPES = new Set([
  "application/json",
  "application/xml",
  "application/javascript",
  "application/ecmascript",
  "application/typescript",
  "application/x-javascript",
  "application/x-typescript",
  "application/x-sh",
  "application/x-shellscript",
  "application/x-csh",
  "application/x-perl",
  "application/x-python",
  "application/x-ruby",
  "application/x-php",
  "application/x-httpd-php",
  "application/sql",
  "application/toml",
  "application/yaml",
  "application/x-yaml",
  "application/x-tex",
  "application/x-latex",
  "application/graphql",
  "application/x-ndjson",
  "application/jsonl",
]);

/**
 * File extensions read as text whatever type the uploader's system gave, which for most source
 * files is none (`application/octet-stream`). Each is also the name its code is highlighted by,
 * which highlight.js resolves through its aliases (`rs`, `py`, `ts`) or leaves plain.
 */
const TEXT_EXTENSIONS = new Set([
  "txt",
  "text",
  "log",
  "md",
  "markdown",
  "rst",
  "adoc",
  "csv",
  "tsv",
  "json",
  "jsonc",
  "json5",
  "jsonl",
  "ndjson",
  "yaml",
  "yml",
  "toml",
  "ini",
  "cfg",
  "conf",
  "properties",
  "env",
  "xml",
  "html",
  "htm",
  "xhtml",
  "svg",
  "css",
  "scss",
  "sass",
  "less",
  "js",
  "mjs",
  "cjs",
  "jsx",
  "ts",
  "mts",
  "cts",
  "tsx",
  "vue",
  "svelte",
  "rs",
  "go",
  "c",
  "h",
  "cc",
  "cpp",
  "cxx",
  "hpp",
  "hh",
  "hxx",
  "m",
  "mm",
  "cs",
  "fs",
  "fsx",
  "java",
  "kt",
  "kts",
  "scala",
  "groovy",
  "gradle",
  "swift",
  "dart",
  "py",
  "pyi",
  "rb",
  "php",
  "pl",
  "pm",
  "lua",
  "r",
  "jl",
  "hs",
  "ml",
  "mli",
  "ex",
  "exs",
  "erl",
  "hrl",
  "clj",
  "cljs",
  "lisp",
  "el",
  "scm",
  "rkt",
  "zig",
  "nim",
  "v",
  "sv",
  "vhd",
  "vhdl",
  "asm",
  "s",
  "sh",
  "bash",
  "zsh",
  "fish",
  "ps1",
  "psm1",
  "bat",
  "cmd",
  "sql",
  "graphql",
  "gql",
  "proto",
  "wit",
  "wat",
  "tex",
  "bib",
  "diff",
  "patch",
  "cmake",
  "mk",
  "nix",
  "tf",
  "hcl",
  "dockerfile",
  "gitignore",
  "gitattributes",
  "editorconfig",
  "srt",
  "vtt",
]);

/** Extensions whose text is prose or data, shown as written without highlighting. */
const PLAIN_EXTENSIONS = new Set(["txt", "text", "log", "csv", "tsv", "srt", "vtt", "env"]);

/** Whole names, without an extension, that name what their code is written in. */
const NAMED_FILES: Readonly<Record<string, string>> = {
  dockerfile: "dockerfile",
  containerfile: "dockerfile",
  makefile: "makefile",
  gnumakefile: "makefile",
  "cmakelists.txt": "cmake",
  gemfile: "ruby",
  rakefile: "ruby",
  justfile: "makefile",
  "build.gradle": "gradle",
};

/**
 * How an attachment shows its first lines: `undefined` when it is not text, `null` when it is
 * text shown as written, and otherwise the name its code is highlighted by. Its declared type
 * decides first, then its name, since most systems declare no type for source files.
 */
export function textLanguage(fileName: string, mimeType: string): string | null | undefined {
  const name = fileName.trim().toLowerCase();
  const named = NAMED_FILES[name];
  if (named !== undefined) {
    return named;
  }
  const dot = name.lastIndexOf(".");
  // A name that is all extension (`.gitignore`) is its own extension.
  const extension = dot === -1 ? "" : name.slice(dot + 1);
  const essence = (mimeType.split(";")[0] ?? "").trim().toLowerCase();
  const textType =
    essence.startsWith("text/") ||
    TEXT_TYPES.has(essence) ||
    essence.endsWith("+json") ||
    essence.endsWith("+xml");
  if (!textType && !TEXT_EXTENSIONS.has(extension)) {
    return undefined;
  }
  if (extension === "" || PLAIN_EXTENSIONS.has(extension)) {
    return null;
  }
  return extension;
}

/** A text attachment's first lines, and whether the file has more than they show. */
export interface TextHead {
  text: string;
  more: boolean;
}

/**
 * The first `maxLines` lines of `text`, which is the whole file when `complete` and otherwise
 * only its beginning, whose last line may be cut short. A file's final line break ends its last
 * line rather than starting another.
 */
export function headLines(text: string, complete: boolean, maxLines: number): TextHead {
  const lines = text.split(/\r\n|\r|\n/);
  if (complete && lines.length > 1 && lines.at(-1) === "") {
    lines.pop();
  }
  return {
    text: lines.slice(0, maxLines).join("\n"),
    more: !complete || lines.length > maxLines,
  };
}

/** The decoder for a text type's `charset`, UTF-8 when it names none this browser knows. */
function decoderFor(mimeType: string): TextDecoder {
  const charset = /;\s*charset\s*=\s*"?([^";\s]+)/i.exec(mimeType)?.[1];
  if (charset !== undefined) {
    try {
      return new TextDecoder(charset);
    } catch {
      // An unknown label is taken as UTF-8, as most text is.
    }
  }
  return new TextDecoder();
}

/** A file that turned out not to be text: it holds a NUL, which no text written to be read does. */
export class NotText extends Error {}

/**
 * Reads the first lines of the text file at `url` from the deployment's storage, which lets any
 * origin read what it serves without credentials, stopping at `TEXT_PREVIEW_MAX_BYTES` or the
 * line past `TEXT_PREVIEW_LINES`, whichever comes first, and cancelling the rest of the download.
 */
async function readHead(url: string, mimeType: string): Promise<TextHead> {
  const abort = new AbortController();
  try {
    const response = await fetch(url, {
      mode: "cors",
      credentials: "omit",
      signal: abort.signal,
    });
    if (!response.ok || response.body === null) {
      throw new Error(`${String(response.status)} ${response.statusText}`);
    }
    const reader = response.body.getReader();
    const decoder = decoderFor(mimeType);
    let text = "";
    let read = 0;
    let complete = false;
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) {
        text += decoder.decode();
        complete = true;
        break;
      }
      const room = TEXT_PREVIEW_MAX_BYTES - read;
      const bytes = chunk.value.byteLength > room ? chunk.value.subarray(0, room) : chunk.value;
      read += bytes.byteLength;
      text += decoder.decode(bytes, { stream: true });
      if (read >= TEXT_PREVIEW_MAX_BYTES || lineCount(text) > TEXT_PREVIEW_LINES) {
        break;
      }
    }
    if (text.includes("\u0000")) {
      throw new NotText();
    }
    return headLines(text, complete, TEXT_PREVIEW_LINES);
  } finally {
    abort.abort();
  }
}

function lineCount(text: string): number {
  let count = 1;
  for (let i = text.indexOf("\n"); i !== -1; i = text.indexOf("\n", i + 1)) {
    count++;
  }
  return count;
}

/** How many files' first lines are kept, so a channel scrolled back through reads none again. */
const KEPT_HEADS = 64;

/** First lines read, by address, the least recently asked for first. */
const heads = new Map<string, TextHead>();
const reading = new Map<string, Promise<TextHead>>();

/** The first lines of the file at `url`, when they have already been read. */
export function keptHead(url: string): TextHead | undefined {
  const head = heads.get(url);
  if (head !== undefined) {
    heads.delete(url);
    heads.set(url, head);
  }
  return head;
}

/**
 * The first lines of the text file at `url`, read once however many messages show it at once,
 * and kept for the `KEPT_HEADS` files asked for most recently. A failed read is not kept, so the
 * next one tries again.
 */
export function textHead(url: string, mimeType: string): Promise<TextHead> {
  const kept = keptHead(url);
  if (kept !== undefined) {
    return Promise.resolve(kept);
  }
  let pending = reading.get(url);
  if (pending === undefined) {
    pending = readHead(url, mimeType)
      .then((head) => {
        heads.set(url, head);
        for (const oldest of heads.keys()) {
          if (heads.size <= KEPT_HEADS) {
            break;
          }
          heads.delete(oldest);
        }
        return head;
      })
      .finally(() => {
        reading.delete(url);
      });
    reading.set(url, pending);
  }
  return pending;
}
