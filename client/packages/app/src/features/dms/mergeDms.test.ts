import { describe, expect, it } from "vitest";
import { mergeDms } from "./mergeDms";

const source = (dms: [string, string | undefined][]) => ({
  dms: dms.map(([id]) => id),
  activity: (id: string) => dms.find(([dm]) => dm === id)?.[1],
});

describe("mergeDms", () => {
  it("orders every deployment's DMs by their newest message, quiet ones last", () => {
    const home = source([
      ["h1", "0190-3"],
      ["h2", "0190-1"],
      ["h3", undefined],
    ]);
    const away = source([
      ["a1", "0190-2"],
      ["a2", undefined],
    ]);
    expect(mergeDms([home, away])).toEqual(["h1", "a1", "h2", "h3", "a2"]);
  });
});
