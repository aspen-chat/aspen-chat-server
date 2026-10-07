import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { externalUrl, isAppPage, permitted } from "./navigation.ts";

void describe("externalUrl", () => {
  void it("passes web and mail links on", () => {
    assert.equal(externalUrl("https://example.org/a?b#c"), "https://example.org/a?b#c");
    assert.equal(externalUrl("http://example.org"), "http://example.org/");
    assert.equal(externalUrl("mailto:someone@example.org"), "mailto:someone@example.org");
  });

  void it("keeps files and other programs' schemes in", () => {
    for (const url of [
      "file://attacker/share/payload.exe",
      "file:///etc/passwd",
      "search-ms:query=x&crumb=location:\\\\attacker\\share",
      "ms-officecmd:{}",
      "javascript:alert(1)",
      "data:text/html,<script>alert(1)</script>",
      "smb://attacker/share",
      "aspen://app/device-link",
      "//attacker/share/payload.exe",
      "/relative",
      "",
    ]) {
      assert.equal(externalUrl(url), null, url);
    }
  });
});

void describe("isAppPage", () => {
  const packaged = { indexUrl: "file:///opt/Aspen/resources/app/index.html" };
  const development = { devServerUrl: "http://localhost:5173" };

  void it("allows the packaged index and its routes", () => {
    assert.ok(isAppPage("file:///opt/Aspen/resources/app/index.html", packaged));
    assert.ok(isAppPage("file:///opt/Aspen/resources/app/index.html#/communities/x", packaged));
  });

  void it("refuses every other file, and files on other machines", () => {
    assert.ok(!isAppPage("file:///opt/Aspen/resources/app/other.html", packaged));
    assert.ok(!isAppPage("file://attacker/opt/Aspen/resources/app/index.html", packaged));
    assert.ok(!isAppPage("file:///opt/Aspen/resources/app/index.html?x=1", packaged));
    assert.ok(!isAppPage("https://example.org/", packaged));
  });

  void it("allows only the dev server's origin in development", () => {
    assert.ok(isAppPage("http://localhost:5173/communities/x", development));
    assert.ok(!isAppPage("http://localhost:5174/", development));
    assert.ok(!isAppPage("http://localhost:5173.attacker.example/", development));
    assert.ok(!isAppPage("file:///etc/passwd", development));
  });
});

void describe("permitted", () => {
  const app = { indexUrl: "file:///opt/Aspen/resources/app/index.html" };
  const page = "file:///opt/Aspen/resources/app/index.html#/communities/x";

  void it("grants the app's page what it uses", () => {
    for (const permission of ["media", "display-capture", "notifications", "fileSystem"]) {
      assert.equal(permitted(permission, { requestingUrl: page }, app), true, permission);
    }
  });

  void it("grants other frames nothing but fullscreen and web and mail links", () => {
    for (const requestingUrl of ["https://player.example/embed", "about:srcdoc", "null"]) {
      for (const permission of ["media", "display-capture", "notifications", "fileSystem"]) {
        assert.equal(permitted(permission, { requestingUrl }, app), false, permission);
      }
      assert.equal(permitted("fullscreen", { requestingUrl }, app), true);
    }
    assert.equal(
      permitted(
        "openExternal",
        { requestingUrl: "about:srcdoc", externalURL: "https://a.example/" },
        app,
      ),
      true,
    );
    for (const externalURL of ["ms-officecmd:{}", "search-ms:query=x", "file:///etc/passwd"]) {
      assert.equal(permitted("openExternal", { requestingUrl: page, externalURL }, app), false);
    }
  });

  void it("refuses what the app does not use, even to its page", () => {
    for (const permission of [
      "geolocation",
      "midi",
      "clipboard-read",
      "hid",
      "serial",
      "unknown",
    ]) {
      assert.equal(permitted(permission, { requestingUrl: page }, app), false, permission);
    }
  });
});
