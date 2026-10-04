import { describe, expect, it } from "vitest";
import { thisDeviceName } from "./deviceName";

const join = (app: string, system: string | null) =>
  system === null ? app : `${app} on ${system}`;

describe("thisDeviceName", () => {
  it("names the browser and system on the web", () => {
    const firefox = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";
    expect(thisDeviceName(join, firefox, "web")).toBe("Firefox on Linux");
    const edge =
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Safari/537.36 Edg/140.0";
    expect(thisDeviceName(join, edge, "web")).toBe("Edge on Windows");
    const safari =
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
    expect(thisDeviceName(join, safari, "web")).toBe("Safari on macOS");
  });

  it("names the app in the desktop and mobile shells", () => {
    const pixel =
      "Mozilla/5.0 (Linux; Android 15; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0 Mobile Safari/537.36";
    expect(thisDeviceName(join, pixel, "mobile")).toBe("Aspen on Android");
    expect(thisDeviceName(join, "Electron Windows NT", "desktop")).toBe("Aspen on Windows");
    expect(thisDeviceName(join, "something else", "desktop")).toBe("Aspen");
  });
});
