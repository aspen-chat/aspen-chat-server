import { expect, test } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * The device lists in Settings against a stand-in for the browser's media devices, which names
 * devices only once a permission is granted and records every request for one: opening Settings
 * outside a call asks for nothing, and each list is filled once the user allows it.
 */

declare global {
  interface Window {
    mediaRequests: string[];
  }
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(() => {
    window.mediaRequests = [];
    const granted = new Set<string>();
    const devices = [
      { deviceId: "m1", kind: "audioinput", label: "Desk microphone", group: "audio" },
      { deviceId: "s1", kind: "audiooutput", label: "Desk speakers", group: "audio" },
      { deviceId: "c1", kind: "videoinput", label: "Desk camera", group: "video" },
    ];
    const mediaDevices = new EventTarget() as EventTarget & Record<string, unknown>;
    mediaDevices.enumerateDevices = () =>
      Promise.resolve(
        devices.map((d) => ({
          deviceId: granted.has(d.group) ? d.deviceId : "",
          kind: d.kind,
          label: granted.has(d.group) ? d.label : "",
          groupId: "",
        })),
      );
    mediaDevices.getUserMedia = (constraints: MediaStreamConstraints) => {
      const group = constraints.audio === undefined ? "video" : "audio";
      window.mediaRequests.push(group);
      granted.add(group);
      return Promise.resolve({ getTracks: () => [] });
    };
    Object.defineProperty(navigator, "mediaDevices", { value: mediaDevices });
  });
});

test("Settings asks for no device permission until the user allows one", async ({ page }) => {
  await signInToWorld(page);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  const allowMicrophone = settings.getByRole("button", { name: "Allow microphone" });
  const allowCamera = settings.getByRole("button", { name: "Allow camera" });
  await expect(allowMicrophone).toBeVisible();
  await expect(allowCamera).toBeVisible();
  expect(await page.evaluate(() => window.mediaRequests)).toEqual([]);

  await allowMicrophone.click();
  await expect(allowMicrophone).toBeHidden();
  await expect(allowCamera).toBeVisible();
  await settings.getByRole("button", { name: "System default Microphone" }).click();
  await expect(page.getByRole("option", { name: "Desk microphone" })).toBeVisible();
  await page.keyboard.press("Escape");

  await allowCamera.click();
  await expect(allowCamera).toBeHidden();
  expect(await page.evaluate(() => window.mediaRequests)).toEqual(["audio", "video"]);
});
