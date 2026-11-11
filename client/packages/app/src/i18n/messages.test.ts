import { parse } from "@formatjs/icu-messageformat-parser";
import { afterEach, describe, expect, it, vi } from "vitest";
import spec from "../../../../../spec/moderation_actions.json";
import { resolveLocale } from "@/i18n/locales";
import { en, format, register, showLanguage, type StringTree } from "@/i18n/messages";

function strings(tree: StringTree, path = ""): [string, string][] {
  return Object.entries(tree).flatMap(([key, value]) =>
    typeof value === "string"
      ? [[`${path}${key}`, value] as [string, string]]
      : strings(value, `${path}${key}.`),
  );
}

describe("messages", () => {
  it("names every action the moderation log records", () => {
    expect(Object.keys(en.admin.moderationActions).sort()).toEqual([...spec.actions].sort());
  });

  it("are all valid MessageFormat", () => {
    const invalid = strings(en).filter(([, template]) => {
      try {
        parse(template, { ignoreTag: true });
        return false;
      } catch {
        return true;
      }
    });
    expect(invalid).toEqual([]);
  });
});

describe("format", () => {
  afterEach(() => {
    showLanguage("en");
  });

  it("chooses a plural's form by its number, writing the number as its language does", () => {
    expect(format(en.poll.votesTotal, { count: 1 })).toBe("1 vote");
    expect(format(en.poll.votesTotal, { count: 1234 })).toBe("1,234 votes");
    expect(format(en.poll.winner, { option: "Tea", count: 0 })).toBe("Tea won with 0 votes.");
  });

  it("takes the plural forms of the catalogue a template is from", () => {
    const polish = {
      files: "{count, plural, one {# plik} few {# pliki} many {# plików} other {# pliku}}",
    };
    register(polish, "pl");
    showLanguage("pl");
    expect([2, 5, 22].map((count) => format(polish.files, { count }))).toEqual([
      "2 pliki",
      "5 plików",
      "22 pliki",
    ]);
  });

  it("reads a text two catalogues hold as the one shown writes it", () => {
    const same = "{count, plural, one {# min} other {# min}}";
    register({ minutes: same }, "pl");
    register({ minutes: same }, "de");
    showLanguage("pl");
    expect(format(same, { count: 12345.5 })).toBe("12\u00a0345,5 min");
    showLanguage("de");
    expect(format(same, { count: 12345.5 })).toBe("12.345,5 min");
    showLanguage("en");
    expect(format(same, { count: 12345.5 })).toBe("12,345.5 min");
  });

  it("writes numbers a translation's way in a template it holds as English does", () => {
    const { language, messages } = resolveLocale("es", []);
    expect(messages.moreImages).toBe(en.moreImages);
    showLanguage(language);
    expect(format(messages.moreImages, { count: 12345 })).toBe("+12.345");
    showLanguage("en");
    expect(format(en.moreImages, { count: 12345 })).toBe("+12,345");
  });

  it("reads a string a translation lacks as English", () => {
    const untranslated = "{count, plural, one {# widget} other {# widgets}}";
    register({ widgets: untranslated }, "en");
    register({}, "fr");
    showLanguage("fr");
    // French counts none as one; English does not.
    expect(format(untranslated, { count: 0 })).toBe("0 widgets");
  });

  it("leaves a plain placeholder it has no value for as written", () => {
    expect(format("From {who} in {where}", { who: "Kate" })).toBe("From Kate in {where}");
  });

  it("shows a template that is not valid MessageFormat as written", () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => undefined);
    expect(format("{count, plural, one {# file}", { count: 2 })).toBe(
      "{count, plural, one {# file}",
    );
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });

  it("reads angle brackets and a lone # as text", () => {
    expect(format("Bearer <token> in #{channel}", { channel: "general" })).toBe(
      "Bearer <token> in #general",
    );
  });
});
