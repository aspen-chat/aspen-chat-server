import { describe, expect, it } from "vitest";
import { decodeTags, encodeTags, tagQueryAt } from "@/features/mentions/tags";

const A = "01a0e95a-8b0f-75df-a00b-29a4f0b878d1";
const B = "01a0e95a-8b0f-75df-a00b-29a4f0b878d2";

describe("tags", () => {
  it("finds the @ word being typed", () => {
    expect(tagQueryAt("hi @ka", 6)).toEqual({ start: 3, query: "ka" });
    expect(tagQueryAt("@", 1)).toEqual({ start: 0, query: "" });
    expect(tagQueryAt("mail@ka", 7)).toBeNull();
    expect(tagQueryAt("hi @kate done", 13)).toBeNull();
  });

  it("sends picked tags as tokens, whole words only, longest first", () => {
    const picks = [
      { text: "@Team", token: `<@&${A}>` },
      { text: "@Team leads", token: `<@&${B}>` },
      { text: "@kate", token: `<@${A}>` },
    ];
    expect(encodeTags("@Team leads and @Team, not @kated, but @kate.", picks)).toBe(
      `<@&${B}> and <@&${A}>, not @kated, but <@${A}>.`,
    );
  });

  it("reads a sent message back as it was written", () => {
    const decoded = decodeTags(
      `<@${A}> and <@&${B}> and <@${B}>`,
      (id) => (id === A ? "kate" : undefined),
      (id) => (id === B ? "Team" : undefined),
    );
    expect(decoded.text).toBe(`@kate and @Team and <@${B}>`);
    expect(encodeTags(decoded.text, decoded.picks)).toBe(`<@${A}> and <@&${B}> and <@${B}>`);
  });
});
