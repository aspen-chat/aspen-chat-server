/**
 * Syntax highlighting for fenced code blocks, loaded on first use.
 *
 * highlight.js's "common" subset (about forty languages) is bundled with this module; every
 * other grammar it ships is a separate chunk fetched the first time a block names it. A fence
 * naming a language highlight.js does not know renders as plain code. Nothing is auto-detected.
 *
 * This module is imported dynamically by `CodeBlock`, so a session that never renders a code
 * block never downloads any of it.
 */

import hljs from "highlight.js/lib/common";
import type { LanguageFn } from "highlight.js";

/**
 * Fence names people use that are not registered aliases of the grammar they mean. Registered
 * aliases (`js`, `ts`, `rb`, `sh`, `yml`, `kt`, `rs`, ...) resolve on their own.
 */
const ALIASES: Readonly<Record<string, string>> = {
  hs: "haskell",
  ex: "elixir",
  exs: "elixir",
  ps1: "powershell",
  docker: "dockerfile",
  proto: "protobuf",
  jl: "julia",
  zsh: "bash",
  toml: "ini",
  htm: "xml",
  html: "xml",
};

/** Every grammar highlight.js ships, keyed by file name, each a chunk loaded on demand. */
const GRAMMARS: Record<string, () => Promise<{ default: LanguageFn }>> = Object.fromEntries(
  Object.entries(
    import.meta.glob<{ default: LanguageFn }>([
      "/node_modules/highlight.js/es/languages/*.js",
      "!/node_modules/highlight.js/es/languages/*.js.js",
    ]),
  ).flatMap(([path, load]) => {
    const match = /\/([a-z0-9_-]+)\.js$/.exec(path);
    return match?.[1] === undefined ? [] : [[match[1], load]];
  }),
);

/** Grammars asked for that highlight.js does not ship, so they are not asked for again. */
const unknown = new Set<string>();
const loading = new Map<string, Promise<boolean>>();

function normalize(language: string): string {
  const lower = language.trim().toLowerCase();
  return ALIASES[lower] ?? lower;
}

/** Whether `language` is highlighted right now, without loading anything. */
export function isReady(language: string): boolean {
  return hljs.getLanguage(normalize(language)) !== undefined;
}

/**
 * Makes `language` available, fetching its grammar if needed. Resolves to `false` when
 * highlight.js has no grammar for it.
 */
export function ensureLanguage(language: string): Promise<boolean> {
  const name = normalize(language);
  if (hljs.getLanguage(name) !== undefined) {
    return Promise.resolve(true);
  }
  if (unknown.has(name)) {
    return Promise.resolve(false);
  }
  const load = GRAMMARS[name];
  if (load === undefined) {
    unknown.add(name);
    return Promise.resolve(false);
  }
  let pending = loading.get(name);
  if (pending === undefined) {
    pending = load()
      .then((grammar) => {
        hljs.registerLanguage(name, grammar.default);
        return true;
      })
      .catch(() => {
        unknown.add(name);
        return false;
      })
      .finally(() => {
        loading.delete(name);
      });
    loading.set(name, pending);
  }
  return pending;
}

/**
 * Highlights `code` as `language`, which must already be available, returning HTML whose only
 * markup is `<span class="hljs-…">`; the code itself is escaped. `null` for an unknown language.
 */
export function highlight(code: string, language: string): string | null {
  const name = normalize(language);
  if (hljs.getLanguage(name) === undefined) {
    return null;
  }
  return hljs.highlight(code, { language: name, ignoreIllegals: true }).value;
}
