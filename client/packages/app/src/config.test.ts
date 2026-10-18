import { allowLoopbackPlainText } from "@aspen/protocol";
import { afterEach, describe, expect, it } from "vitest";
import { defaultServerUrl, rememberServerUrl } from "./config";

describe("defaultServerUrl", () => {
  afterEach(() => {
    window.localStorage.clear();
    allowLoopbackPlainText(false);
  });

  it("uses the page's origin on the web, whatever was remembered", () => {
    // The test page is served over plain HTTP at this machine, as a development build's is.
    allowLoopbackPlainText(true);
    expect(defaultServerUrl("web")).toBe(window.location.origin);
    rememberServerUrl("https://chat.example.org");
    expect(defaultServerUrl("web")).toBe(window.location.origin);
  });

  it("uses no plain HTTP origin outside a development build", () => {
    expect(new URL(window.location.origin).protocol).toBe("http:");
    expect(defaultServerUrl("web")).toBeNull();
  });

  it("uses the remembered server in a shell without a meaningful origin", () => {
    expect(defaultServerUrl("desktop")).toBeNull();
    rememberServerUrl("https://chat.example.org");
    expect(defaultServerUrl("desktop")).toBe("https://chat.example.org");
    expect(defaultServerUrl("mobile")).toBe("https://chat.example.org");
  });

  it("forgets a server remembered over plain HTTP, so the person is asked again", () => {
    rememberServerUrl("http://192.168.1.20:8000");
    expect(defaultServerUrl("mobile")).toBeNull();
    expect(defaultServerUrl("desktop")).toBeNull();
  });
});
