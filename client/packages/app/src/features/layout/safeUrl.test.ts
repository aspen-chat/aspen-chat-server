import { describe, expect, it } from "vitest";
import { mediaUrlOnPage, messageLinkUrl, webPageUrl } from "@/features/layout/safeUrl";

const UNSAFE = [
  "javascript:alert(1)",
  "JavaScript:alert(1)",
  "data:text/html,<script>alert(1)</script>",
  "file://attacker/share/payload.exe",
  "file:///etc/passwd",
  "//attacker/share/payload.exe",
  "/relative/path",
  "relative",
  "search-ms:query=x",
  "ms-officecmd:{}",
  "blob:https://example.org/x",
  "",
];

describe("safe URLs", () => {
  it("open web pages, over http too", () => {
    expect(webPageUrl("https://example.org/a")).toBe("https://example.org/a");
    expect(webPageUrl("http://example.org")).toBe("http://example.org/");
    expect(webPageUrl("mailto:a@example.org")).toBeUndefined();
    expect(webPageUrl(null)).toBeUndefined();
    expect(webPageUrl(undefined)).toBeUndefined();
  });

  it("link messages to web pages and mail addresses", () => {
    expect(messageLinkUrl("https://example.org/")).toBe("https://example.org/");
    expect(messageLinkUrl("mailto:a@example.org")).toBe("mailto:a@example.org");
  });

  it("refuse every other scheme, and every relative address", () => {
    for (const url of UNSAFE) {
      expect(webPageUrl(url), url).toBeUndefined();
      expect(messageLinkUrl(url), url).toBeUndefined();
      expect(mediaUrlOnPage(url, "https:"), url).toBeUndefined();
      expect(mediaUrlOnPage(url, "http:"), url).toBeUndefined();
    }
  });

  it("load media over https, and over http only in development", () => {
    expect(mediaUrlOnPage("https://media.example.org/a.webp", "https:")).toBe(
      "https://media.example.org/a.webp",
    );
    expect(mediaUrlOnPage("http://media.example.org/a.webp", "https:")).toBeUndefined();
    expect(mediaUrlOnPage("http://media.example.org/a.webp", "file:")).toBeUndefined();
    expect(mediaUrlOnPage("http://media.example.org/a.webp", "http:")).toBe(
      "http://media.example.org/a.webp",
    );
    for (const url of [
      "http://localhost:9000/a",
      "http://alpha.localhost/a",
      "http://127.0.0.1:9000/a",
      "http://[::1]:9000/a",
    ]) {
      expect(mediaUrlOnPage(url, "https:"), url).toBe(url);
    }
  });
});
