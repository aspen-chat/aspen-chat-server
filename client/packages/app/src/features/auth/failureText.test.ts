import { ApiProblemError, DevicePuzzleRefusedError, UnreachableError } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { en } from "@/i18n/messages";
import { failureText } from "./failureText";

describe("failureText", () => {
  it("says a deployment that never answered could not be reached, naming its host", () => {
    const text = failureText(
      new UnreachableError("https://chat.example.org:8443", { cause: new TypeError("x") }),
      en,
    );
    expect(text).toBe(
      "chat.example.org:8443 couldn't be reached. Check your connection, then try again.",
    );
  });

  it("says the same of a deployment whose device puzzle was given up on", () => {
    expect(
      failureText(new DevicePuzzleRefusedError("https://chat.example.org", "too hard"), en),
    ).toBe("chat.example.org couldn't be reached. Check your connection, then try again.");
  });

  it("gives a refusal in the server's own words", () => {
    const refusal = new ApiProblemError({
      code: "rateLimited",
      title: "Too many requests",
      detail: "Slow down, then try again in a minute.",
      status: 429,
    });
    expect(failureText(refusal, en)).toBe("Slow down, then try again in a minute.");
  });
});
