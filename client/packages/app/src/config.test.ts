import { afterEach, describe, expect, it } from "vitest";
import { defaultServerUrl, rememberServerUrl } from "./config";

describe("defaultServerUrl", () => {
  afterEach(() => {
    window.localStorage.clear();
  });

  it("prefers a remembered server, then falls back to the page origin on the web", () => {
    expect(defaultServerUrl("web")).toBe(window.location.origin);
    rememberServerUrl("https://chat.example.org");
    expect(defaultServerUrl("web")).toBe("https://chat.example.org");
  });

  it("has no default in a shell without a meaningful origin", () => {
    expect(defaultServerUrl("desktop")).toBeNull();
  });
});
