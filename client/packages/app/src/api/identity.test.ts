import { describe, expect, it } from "vitest";
import { identityOf } from "./identity";

describe("identityOf", () => {
  it("names one person the same on every deployment", () => {
    const atHome = identityOf(
      { id: "a1", homeDomain: null, homeId: null },
      "b.example",
      "a.example",
    );
    const abroad = identityOf(
      { id: "z9", homeDomain: "b.example", homeId: "a1" },
      null,
      "a.example",
    );
    expect(atHome).toBe("b.example/a1");
    expect(abroad).toBe(atHome);
    expect(identityOf({ id: "h1", homeDomain: null, homeId: null }, null, "a.example")).toBe(
      "a.example/h1",
    );
  });
});
