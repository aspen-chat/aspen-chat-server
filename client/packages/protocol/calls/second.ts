/**
 * One person in a call from Chromium through `page.ts`, driven over standard input, for a check
 * whose other participant is elsewhere (`scripts/check_mobile_call.py`, where it is a phone's
 * simulator or emulator). Joins the call with a WAV played as the microphone, prints `connected` once the
 * call is on, then answers each line read: `report` prints the page's `CallReport` as one line
 * of JSON, `loudness` the loudest sample its audio slots carried over a second (the page's
 * `loudness`, since the browser's own level statistics read zero), and `leave` leaves and exits.
 *
 * ASPEN_CALL_SECOND='{"base": …, "channel": …, "session": <session>, "tone": "tone.wav"}' tsx calls/second.ts
 */

import { chromium } from "@playwright/test";
import { createInterface } from "node:readline";
import type { Session } from "../src";
import type { CallReport } from "./page";

interface Arguments {
  readonly base: string;
  readonly channel: string;
  readonly session: Session;
  /** A WAV file Chromium captures as the microphone, in a loop. */
  readonly tone: string;
}

const args = JSON.parse(process.env.ASPEN_CALL_SECOND ?? "{}") as Arguments;

const browser = await chromium.launch({
  args: [
    "--use-fake-device-for-media-stream",
    "--use-fake-ui-for-media-stream",
    `--use-file-for-fake-audio-capture=${args.tone}`,
    "--autoplay-policy=no-user-gesture-required",
  ],
});
const context = await browser.newContext({ permissions: ["microphone"] });
const page = await context.newPage();
page.on("pageerror", (error) => {
  console.error(`page: ${error.message}`);
});
await page.goto(`${args.base}/`);
await page.waitForFunction(() => window.aspenCall !== undefined);
await page.evaluate(
  async ([session, channel]) => {
    await window.aspenCall?.join(session, channel);
  },
  [args.session, args.channel] as const,
);

function report(): Promise<CallReport> {
  return page.evaluate(async () => {
    const reported = await window.aspenCall?.report();
    if (reported === undefined) {
      throw new Error("no call");
    }
    return reported;
  });
}

const deadline = Date.now() + 30_000;
let last = await report();
while (!(last.status === "connected" && last.connection === "connected")) {
  if (Date.now() > deadline) {
    console.error(`not connected: ${JSON.stringify(last)}`);
    await browser.close();
    process.exit(1);
  }
  await new Promise((resolve) => setTimeout(resolve, 250));
  last = await report();
}
console.log("connected");

const lines = createInterface({ input: process.stdin });
for await (const line of lines) {
  if (line.trim() === "report") {
    console.log(JSON.stringify(await report()));
  } else if (line.trim() === "loudness") {
    console.log(JSON.stringify(await page.evaluate(() => window.aspenCall?.loudness() ?? 0)));
  } else if (line.trim() === "leave") {
    break;
  }
}
await page.evaluate(() => {
  window.aspenCall?.leave();
});
await browser.close();
process.exit(0);
