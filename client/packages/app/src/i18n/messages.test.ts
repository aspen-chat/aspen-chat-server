import { parse } from "@formatjs/icu-messageformat-parser";
import { describe, expect, it, vi } from "vitest";
import spec from "../../../../../spec/moderation_actions.json";
import { en, format, register, type StringTree } from "@/i18n/messages";

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
    expect([2, 5, 22].map((count) => format(polish.files, { count }))).toEqual([
      "2 pliki",
      "5 plików",
      "22 pliki",
    ]);
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
