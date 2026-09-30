import { describe, expect, it } from "vitest";
import { callLength } from "./callLength";

describe("callLength", () => {
  it("names each part in full, joined as a list, leaving out the zero ones", () => {
    expect(callLength("en", 0)).toBe("0 seconds");
    expect(callLength("en", 1)).toBe("1 second");
    expect(callLength("en", 45)).toBe("45 seconds");
    expect(callLength("en", 60)).toBe("1 minute");
    expect(callLength("en", 125)).toBe("2 minutes and 5 seconds");
    expect(callLength("en", 3600)).toBe("1 hour");
  });

  it("leaves seconds out once a call reached an hour", () => {
    expect(callLength("en", 3600 + 5 * 60 + 30)).toBe("1 hour and 5 minutes");
    expect(callLength("en", 2 * 3600 + 59)).toBe("2 hours");
  });

  it("speaks the locale it is given", () => {
    expect(callLength("de", 125)).toBe("2 Minuten und 5 Sekunden");
  });
});
