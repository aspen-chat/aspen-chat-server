import { afterEach, describe, expect, it } from "vitest";
import { defaultServerUrl, rememberServerUrl } from "./config";

describe("defaultServerUrl", () => {
  afterEach(() => {
    window.localStorage.clear();
  });

  it("uses the page's origin on the web, whatever was remembered", () => {
    expect(defaultServerUrl("web")).toBe(window.location.origin);
    rememberServerUrl("https://chat.example.org");
    expect(defaultServerUrl("web")).toBe(window.location.origin);
  });

  it("uses the remembered server in a shell without a meaningful origin", () => {
    expect(defaultServerUrl("desktop")).toBeNull();
    rememberServerUrl("https://chat.example.org");
    expect(defaultServerUrl("desktop")).toBe("https://chat.example.org");
    expect(defaultServerUrl("mobile")).toBe("https://chat.example.org");
  });
});
