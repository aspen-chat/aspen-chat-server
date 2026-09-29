import { describe, expect, it } from "vitest";
import vectors from "../../../../../spec/pseudo_locale_vectors.json";
import { accented, mirrored, pseudoCatalogue } from "./pseudo";

/** The vectors write placeholders as the server does, `%{name}`; the client's are `{name}`. */
const ours = (text: string) => text.replace(/%\{(\w+)\}/g, "{$1}");

describe("pseudo-locales", () => {
  it("pass the vectors the server passes", () => {
    for (const { input, output } of vectors.accented) {
      expect(accented(ours(input))).toBe(ours(output));
    }
    for (const { input, output } of vectors.mirrored) {
      expect(mirrored(ours(input))).toBe(ours(output));
    }
  });

  it("make every string of a catalogue, keeping its shape", () => {
    expect(pseudoCatalogue({ a: "Hi", b: { c: "{n} ok" } }, accented)).toEqual({
      a: "[Ĥîî]",
      b: { c: "[{n} ööķ]" },
    });
  });
});
