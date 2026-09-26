import { describe, expect, it } from "vitest";
import { eventStreamUrl, normalizeServerUrl } from "../src";

describe("normalizeServerUrl", () => {
  it("defaults to https and keeps only the origin", () => {
    expect(normalizeServerUrl("chat.example.org")).toBe("https://chat.example.org");
    expect(normalizeServerUrl(" https://chat.example.org/some/path/ ")).toBe(
      "https://chat.example.org",
    );
    expect(normalizeServerUrl("http://localhost:8080")).toBe("http://localhost:8080");
  });

  it("rejects non-http schemes", () => {
    expect(() => normalizeServerUrl("ftp://example.org")).toThrow(TypeError);
  });
});

describe("eventStreamUrl", () => {
  it("maps http(s) to ws(s) and appends the event stream path", () => {
    expect(eventStreamUrl("https://chat.example.org")).toBe("wss://chat.example.org/api/v1/events");
    expect(eventStreamUrl("http://localhost:8080")).toBe("ws://localhost:8080/api/v1/events");
  });
});
