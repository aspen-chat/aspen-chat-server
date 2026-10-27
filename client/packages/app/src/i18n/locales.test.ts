import { describe, expect, it } from "vitest";
import en from "./locales/en.json";
import es from "./locales/es.json";
import { type StringTree } from "./messages";
import { BETA, LANGUAGES, NATIVE_NAMES, resolveLocale, translated } from "./locales";

describe("resolveLocale", () => {
  it("follows the platform's languages when automatic", () => {
    const british = resolveLocale("automatic", ["en-GB", "en"]);
    expect([british.language, british.locale, british.languages]).toEqual([
      "en",
      "en-GB",
      ["en-GB", "en"],
    ]);
    expect(resolveLocale("automatic", ["de-DE", "en-US"]).locale).toBe("en-US");
    expect(resolveLocale("automatic", ["de-DE"]).locale).toBe("en");
    expect(resolveLocale("automatic", ["en-XA"]).language).toBe("en-XA");
    expect(resolveLocale("automatic", []).language).toBe("en");
    const mexican = resolveLocale("automatic", ["es-MX", "en-US"]);
    expect([mexican.language, mexican.locale, mexican.languages]).toEqual([
      "es",
      "es-MX",
      ["es-MX", "es"],
    ]);
    expect(resolveLocale("automatic", ["es-MX"]).messages.settings.language).toBe("Idioma");
  });

  it("names every beta translation and every native name among the languages", () => {
    for (const language of [...BETA, ...Object.keys(NATIVE_NAMES)]) {
      expect(LANGUAGES).toContain(language);
    }
  });

  it("shows the language chosen, formatting in the platform's variant of it", () => {
    expect(resolveLocale("en", ["en-AU"]).locale).toBe("en-AU");
    const mirrored = resolveLocale("ar-XB", ["en-US"]);
    expect([mirrored.locale, mirrored.direction, mirrored.languages]).toEqual([
      "ar-XB",
      "rtl",
      ["ar-XB"],
    ]);
    expect(resolveLocale("en-XA", ["en-US"]).messages.appName).toBe("[ÅÅšþééñ]");
    expect(resolveLocale("unknown", ["en-US"]).language).toBe("en");
  });
});

/** Each string's path and text, in order. */
function strings(tree: StringTree, prefix = ""): [string, string][] {
  return Object.entries(tree).flatMap(([key, value]) => {
    const path = prefix === "" ? key : `${prefix}.${key}`;
    return typeof value === "string" ? [[path, value]] : strings(value, path);
  });
}

/** The arguments a template names (a name before `,` or `}`, not a branch's text), sorted. */
const placeholders = (template: string) =>
  [...new Set([...template.matchAll(/\{\s*([A-Za-z0-9_]+)\s*[,}]/g)].map((m) => m[1]))].sort();

describe("the translations", () => {
  const english = new Map(strings(en));
  it.each([["es", es]])("%s has only English's keys, with English's placeholders", (_, tree) => {
    for (const [path, text] of strings(tree)) {
      const source = english.get(path);
      expect(source, path).toBeDefined();
      expect(placeholders(text), path).toEqual(placeholders(source ?? ""));
    }
  });
});

describe("translated", () => {
  it("reads in English wherever a translation has nothing, or nothing yet", () => {
    const english = { a: "Hi", b: { c: "Bye", d: "Later" }, e: { f: "Yes" } };
    expect(translated(english, { a: "Salut", b: { c: "" } })).toEqual({
      a: "Salut",
      b: { c: "Bye", d: "Later" },
      e: { f: "Yes" },
    });
  });
});
