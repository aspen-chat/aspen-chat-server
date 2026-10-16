import { describe, expect, it } from "vitest";
import { resolveLocale, translated } from "./locales";

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
