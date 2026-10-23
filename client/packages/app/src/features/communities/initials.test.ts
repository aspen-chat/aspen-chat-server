import { describe, expect, it } from "vitest";
import { initials } from "./initials";

describe("initials", () => {
  it("takes the first and last words' first letters", () => {
    expect(initials("Mira Okonkwo")).toBe("MO");
    expect(initials("  ada  ")).toBe("A");
    expect(initials("")).toBe("?");
  });

  it("keeps each initial a whole character", () => {
    expect(initials("Sam 🦊")).toBe("S🦊");
    expect(initials("👨‍👩‍👧 family")).toBe("👨‍👩‍👧F");
    expect(initials("🇯🇵 Tokyo")).toBe("🇯🇵T");
    expect(initials("émile zola")).toBe("ÉZ");
    expect(initials("高橋 由紀")).toBe("高由");
  });
});
