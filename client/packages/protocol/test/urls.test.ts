import { afterEach, describe, expect, it } from "vitest";
import {
  InsecureAddressError,
  allowLoopbackPlainText,
  eventStreamUrl,
  normalizeServerUrl,
  securelyReached,
} from "../src";

afterEach(() => {
  allowLoopbackPlainText(false);
});

describe("normalizeServerUrl", () => {
  it("defaults to https and keeps only the origin", () => {
    expect(normalizeServerUrl("chat.example.org")).toBe("https://chat.example.org");
    expect(normalizeServerUrl(" https://chat.example.org/some/path/ ")).toBe(
      "https://chat.example.org",
    );
  });

  it("drops everything after the host", () => {
    expect(normalizeServerUrl("chat.example.org:8443/invite/abc?at=x#top")).toBe(
      "https://chat.example.org:8443",
    );
    expect(normalizeServerUrl("chat.example.org?x=1")).toBe("https://chat.example.org");
    expect(normalizeServerUrl("https://user:secret@chat.example.org/")).toBe(
      "https://chat.example.org",
    );
  });

  it("rejects non-http schemes", () => {
    expect(() => normalizeServerUrl("ftp://example.org")).toThrow(TypeError);
  });

  it("refuses plain http, at this machine only where a development build allows it", () => {
    expect(() => normalizeServerUrl("http://chat.example.org")).toThrow(InsecureAddressError);
    expect(() => normalizeServerUrl("http://192.168.1.20:8000")).toThrow(InsecureAddressError);
    expect(() => normalizeServerUrl("http://localhost:8080")).toThrow(InsecureAddressError);
    allowLoopbackPlainText(true);
    expect(normalizeServerUrl("http://localhost:8080")).toBe("http://localhost:8080");
    expect(() => normalizeServerUrl("http://192.168.1.20:8000")).toThrow(InsecureAddressError);
  });
});

describe("eventStreamUrl", () => {
  it("maps https to wss and appends the event stream path", () => {
    expect(eventStreamUrl("https://chat.example.org")).toBe("wss://chat.example.org/api/v1/events");
  });

  it("maps http to ws only where plain text may reach this machine", () => {
    expect(() => eventStreamUrl("http://localhost:8080")).toThrow(InsecureAddressError);
    allowLoopbackPlainText(true);
    expect(eventStreamUrl("http://localhost:8080")).toBe("ws://localhost:8080/api/v1/events");
  });
});

describe("securelyReached", () => {
  it("takes https and wss anywhere, and http and ws at loopback only where allowed", () => {
    for (const url of ["https://chat.example.org", "wss://voice.example.org/ws"]) {
      expect(securelyReached(url), url).toBe(true);
    }
    const loopback = [
      "http://localhost:5173",
      "http://alpha.localhost",
      "ws://127.0.0.1:9001/ws",
      "http://[::1]:8000",
    ];
    for (const url of loopback) {
      expect(securelyReached(url), url).toBe(false);
    }
    allowLoopbackPlainText(true);
    for (const url of loopback) {
      expect(securelyReached(url), url).toBe(true);
    }
    for (const url of [
      "http://chat.example.org",
      "ws://10.0.0.2/ws",
      "ftp://localhost",
      "nonsense",
    ]) {
      expect(securelyReached(url), url).toBe(false);
    }
  });
});
