import { describe, expect, it } from "vitest";
import { allowLoopbackPlainText } from "@aspen/protocol";
import { mailtoUrl, mediaUrl, messageLinkUrl, webPageUrl } from "@/features/layout/safeUrl";

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
      expect(mediaUrl(url), url).toBeUndefined();
    }
  });

  it("load media over https, and over http only from this machine in a development build", () => {
    expect(mediaUrl("https://media.example.org/a.webp")).toBe("https://media.example.org/a.webp");
    const loopback = [
      "http://localhost:9000/a",
      "http://alpha.localhost/a",
      "http://127.0.0.1:9000/a",
      "http://[::1]:9000/a",
    ];
    for (const url of ["http://media.example.org/a.webp", "http://192.168.1.20/a", ...loopback]) {
      expect(mediaUrl(url), url).toBeUndefined();
    }
    allowLoopbackPlainText(true);
    try {
      for (const url of loopback) {
        expect(mediaUrl(url), url).toBe(url);
      }
      expect(mediaUrl("http://media.example.org/a.webp")).toBeUndefined();
      expect(mediaUrl("wss://media.example.org/a")).toBeUndefined();
    } finally {
      allowLoopbackPlainText(false);
    }
  });

  it("write a mail address alone into a mailto link", () => {
    expect(mailtoUrl("kate@example.org")).toBe("mailto:kate@example.org");
    expect(mailtoUrl("kate@example.org?bcc=eve@evil.example&body=hi")).toBe(
      "mailto:kate@example.org%3Fbcc%3Deve@evil.example%26body%3Dhi",
    );
    expect(mailtoUrl("a@example.org,b@example.org")).toBe("mailto:a@example.org%2Cb@example.org");
  });
});
