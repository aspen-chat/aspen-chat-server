import { afterEach, describe, expect, it, vi } from "vitest";
import { mergeDms } from "./mergeDms";

const source = (dms: [string, string | undefined][]) => ({
  dms: dms.map(([id]) => id),
  activity: (id: string) => dms.find(([dm]) => dm === id)?.[1],
});

afterEach(() => {
  vi.restoreAllMocks();
});

describe("mergeDms", () => {
  it("orders ids the same on a device whose language sorts their letters apart", () => {
    // Danish sorts `aa` as one letter after `z`, and ids spell it.
    const danish = new Intl.Collator("da");
    expect(danish.compare("0190-aa", "0190-ab")).toBeGreaterThan(0);
    vi.spyOn(String.prototype, "localeCompare").mockImplementation(function (
      this: string,
      other: string,
    ) {
      return danish.compare(this, other);
    });
    const home = source([["h1", "0190-aa"]]);
    const away = source([["a1", "0190-ab"]]);
    expect(mergeDms([home, away])).toEqual(["a1", "h1"]);
  });

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
