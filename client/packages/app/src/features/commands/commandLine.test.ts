import type { Command } from "@aspen/protocol";
import { describe as group, expect, it } from "vitest";
import {
  commandNameAt,
  describe,
  messageIdIn,
  offeredCommands,
  parameterAt,
  quoteArgument,
  readCommandLine,
  splitArguments,
  tokenize,
} from "./commandLine";

const roll: Command = {
  name: "roll",
  description: "Rolls dice",
  parameters: [
    { name: "dice", description: "Like 2d6", type: "regex", pattern: "[0-9]+d[0-9]+" },
    { name: "note", description: "Why", type: "any", optional: true },
  ],
};
const kick: Command = {
  name: "kick",
  description: "Removes someone",
  parameters: [
    { name: "who", description: "Whom", type: "userId" },
    { name: "proof", description: "A file", type: "attachmentId", optional: true },
    { name: "reason", description: "Why", type: "regex", pattern: "\\w+", optional: true },
  ],
};
const offered = offeredCommands([
  { bot: "dicebot", commands: [roll, kick] },
  { bot: "modbot", commands: [{ ...kick, description: "Kicks" }] },
]);
const usernames: Record<string, string> = { dicebot: "Dice", modbot: "Mod" };
const usernameOf = (bot: string) => usernames[bot];

group("command lines", () => {
  it("reads the name being typed only while the caret is in the first word", () => {
    expect(commandNameAt("/ro", 3)).toEqual({ query: "ro" });
    expect(commandNameAt("/", 1)).toEqual({ query: "" });
    expect(commandNameAt("/roll 2d6", 9)).toBeNull();
    expect(commandNameAt("hi /ro", 6)).toBeNull();
  });

  it("splits words at whitespace outside quotes, reading escapes inside them", () => {
    expect(tokenize('a "b c" "d \\" e" f\\g').map((t) => t.value)).toEqual([
      "a",
      "b c",
      'd " e',
      "f\\g",
    ]);
    expect(tokenize('x "open quote').map((t) => t.value)).toEqual(["x", "open quote"]);
  });

  it("quotes what would split, as the server writes it", () => {
    expect(quoteArgument("2d6")).toBe("2d6");
    expect(quoteArgument("for luck")).toBe('"for luck"');
    expect(quoteArgument('a "b"')).toBe('"a \\"b\\""');
    expect(quoteArgument("")).toBe('""');
  });

  it("names the one bot that answers, or asks which when several do", () => {
    expect(readCommandLine("hello", offered, usernameOf)).toBeNull();
    expect(readCommandLine("/ROLL 2d6", offered, usernameOf)).toMatchObject({
      kind: "command",
      bot: "dicebot",
      argumentsAt: 5,
    });
    expect(readCommandLine("/kick @x", offered, usernameOf)).toEqual({
      kind: "ambiguous",
      name: "kick",
      bots: ["dicebot", "modbot"],
    });
    expect(readCommandLine("/kick @mod @x", offered, usernameOf)).toMatchObject({
      kind: "command",
      bot: "modbot",
      argumentsAt: 10,
    });
    expect(readCommandLine("/nope", offered, usernameOf)).toEqual({
      kind: "unknown",
      name: "nope",
    });
    expect(readCommandLine("/ roll", offered, usernameOf)).toEqual({ kind: "unknown", name: "" });
  });

  it("gives the last parameter that takes any text the rest of the line", () => {
    expect(splitArguments("/roll 2d6  for  luck ", 5, roll)).toEqual({
      kind: "ok",
      values: ["2d6", "for  luck"],
    });
    expect(splitArguments('/roll 2d6 "for luck"', 5, roll)).toEqual({
      kind: "ok",
      values: ["2d6", "for luck"],
    });
    expect(splitArguments("/roll", 5, roll)).toEqual({ kind: "ok", values: [] });
  });

  it("refuses more arguments than parameters, leaving files out of the count", () => {
    expect(splitArguments("/kick a b", 5, kick)).toEqual({ kind: "ok", values: ["a", "b"] });
    expect(splitArguments("/kick a b c", 5, kick)).toEqual({ kind: "tooMany" });
  });

  it("finds the parameter at the caret", () => {
    expect(parameterAt("/roll ", 5, 6, roll)).toMatchObject({ index: 0, token: { value: "" } });
    expect(parameterAt("/roll 2d", 5, 8, roll)).toMatchObject({ index: 0, token: { value: "2d" } });
    expect(parameterAt("/roll 2d6 for", 5, 13, roll)).toMatchObject({ index: 1 });
    expect(parameterAt("/roll 2d6 for luck", 5, 18, roll)).toMatchObject({ index: 1 });
    expect(parameterAt("/kick a b ", 5, 10, kick)).toBeNull();
    expect(parameterAt("/roll", 5, 3, roll)).toBeNull();
  });

  it("reads a message from its link or its id", () => {
    const id = "01a0da3e-2af4-7060-87e8-74700d1bb926";
    expect(messageIdIn(`http://x/communities/a/channels/b/messages/${id}`)).toBe(id);
    expect(messageIdIn(id.toUpperCase())).toBe(id);
    expect(messageIdIn("not a link")).toBeNull();
  });

  it("describes in the reader's language where the bot gave one", () => {
    const item = { description: "Rolls", descriptions: { "fr-CA": "Lance", de: "Würfelt" } };
    expect(describe(item, "fr-CA")).toBe("Lance");
    expect(describe(item, "fr-FR")).toBe("Lance");
    expect(describe(item, "de-AT")).toBe("Würfelt");
    expect(describe(item, "en-GB")).toBe("Rolls");
  });
});
