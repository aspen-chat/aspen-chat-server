import { describe, expect, it } from "vitest";
import vectors from "../../../../../../spec/bot_command_pattern_vectors.json";
import { matchesPattern } from "./commandLine";
import { compilePattern } from "./pattern";

describe("a regex parameter's pattern", () => {
  it("matches the values the server matches", () => {
    const wrong: string[] = [];
    for (const { pattern, matches, rejects } of vectors.accepted) {
      const compiled = compilePattern(pattern);
      if (compiled === null) {
        wrong.push(`${JSON.stringify(pattern)} is not read`);
        continue;
      }
      for (const [values, expected] of [
        [matches, true],
        [rejects, false],
      ] as const) {
        for (const value of values) {
          if (compiled.test(value) !== expected) {
            wrong.push(
              `${JSON.stringify(pattern)} on ${JSON.stringify(value)} is not ${String(expected)}`,
            );
          }
        }
      }
    }
    expect(wrong).toEqual([]);
  });

  it("is left to the server when the server would refuse it", () => {
    const read = vectors.refused.filter(({ pattern }) => compilePattern(pattern) !== null);
    expect(read.map(({ pattern }) => pattern)).toEqual([]);
    expect(matchesPattern("(?i)x", "anything")).toBe(true);
  });

  it("compiles every property name the server takes", () => {
    const { generalCategories, binaryProperties, scripts, scriptsOfUnicode16 } = vectors.properties;
    // An engine older than Unicode 16 lacks its scripts; a pattern naming one is left to the
    // server there.
    const unicode = Number.parseFloat(process.versions.unicode ?? "0");
    const known = scripts.filter((name) => unicode >= 16 || !scriptsOfUnicode16.includes(name));
    const patterns = [
      ...generalCategories.flatMap((name) => [
        `\\p{${name}}`,
        `\\P{gc=${name}}`,
        `[\\p{General_Category=${name}}]`,
      ]),
      ...binaryProperties.map((name) => `\\p{${name}}`),
      ...known.flatMap((name) =>
        ["Script", "sc", "Script_Extensions", "scx"].map((property) => `\\p{${property}=${name}}`),
      ),
    ];
    expect(patterns.filter((pattern) => compilePattern(pattern) === null)).toEqual([]);
  });

  it("is rewritten by its structure, not its text", () => {
    // An escaped backslash before `d` is a backslash and a `d`.
    expect(matchesPattern("\\\\d", "\\d")).toBe(true);
    expect(matchesPattern("\\\\d", "5")).toBe(false);
    // A dot inside a class is a dot.
    expect(matchesPattern("[.]", "a")).toBe(false);
    // A group the pattern never opened is not closed against the wrapper's.
    expect(compilePattern("a)|(b")).toBeNull();
  });
});
