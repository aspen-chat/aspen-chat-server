/**
 * One person in a call from Chromium through `page.ts`, driven over standard input, for a check
 * whose other participant is elsewhere (`scripts/check_mobile_call.py`, where it is a phone's
 * simulator or emulator). Joins the call with a WAV played as the microphone, prints `connected` once the
 * call is on, then answers each line read: `report` prints the page's `CallReport` as one line
 * of JSON, `loudness` the loudest sample its audio slots carried over a second (the page's
 * `loudness`, since the browser's own level statistics read zero), `show` starts showing others'
 * video (`showVideo`), `camera on` and `camera off` turn Chromium's fake camera on and off,
 * `sample <user>` prints a `CameraSample` of that user's camera as the page plays it (or `null`),
 * `offer <json>` offers a file of `size` bytes as `name` (`{"name", "size", "allowDirect"}`) and
 * prints its id and the SHA-256 of what was offered, `files` prints the page's `FilesReport`,
 * `accept <offer> <mode>` accepts an offer into memory, `received <offer> <peer>` prints the
 * SHA-256 of the file received, or `null` while it has not all arrived, each printing `ok` or
 * the JSON, and `leave` leaves and exits.
 *
 * ASPEN_CALL_SECOND='{"base": …, "channel": …, "session": <session>, "tone": "tone.wav"}' tsx calls/second.ts
 */

import { chromium } from "@playwright/test";
import { createInterface } from "node:readline";
import type { Session, TransferMode } from "../src";
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
  const [command, ...rest] = line.trim().split(" ");
  if (command === "report") {
    console.log(JSON.stringify(await report()));
  } else if (command === "loudness") {
    console.log(JSON.stringify(await page.evaluate(() => window.aspenCall?.loudness() ?? 0)));
  } else if (command === "show") {
    await page.evaluate(() => {
      window.aspenCall?.showVideo();
    });
    console.log("ok");
  } else if (command === "camera" && rest[0] === "on") {
    await page.evaluate(() => window.aspenCall?.startCamera());
    console.log("ok");
  } else if (command === "camera" && rest[0] === "off") {
    await page.evaluate(() => {
      window.aspenCall?.stopCamera();
    });
    console.log("ok");
  } else if (command === "sample") {
    const user = rest[0] ?? "";
    console.log(
      JSON.stringify(
        await page.evaluate((who) => window.aspenCall?.cameraSample(who) ?? null, user),
      ),
    );
  } else if (command === "offer") {
    const asked = JSON.parse(rest.join(" ")) as {
      name: string;
      size: number;
      allowDirect: boolean;
    };
    console.log(
      JSON.stringify(
        await page.evaluate(
          ([name, size, allowDirect]) => window.aspenCall?.offerFile(name, size, allowDirect),
          [asked.name, asked.size, asked.allowDirect] as const,
        ),
      ),
    );
  } else if (command === "files") {
    console.log(JSON.stringify(await page.evaluate(() => window.aspenCall?.files())));
  } else if (command === "accept") {
    const [offer, mode] = rest;
    await page.evaluate(
      ([offer, mode]) => {
        window.aspenCall?.acceptOffer(offer, mode);
      },
      [offer ?? "", (mode ?? "directPreferred") as TransferMode] as const,
    );
    console.log("{}");
  } else if (command === "received") {
    const [offer, peer] = rest;
    console.log(
      JSON.stringify({
        sha256: await page.evaluate(
          ([offer, peer]) => window.aspenCall?.receivedHash(offer, peer),
          [offer ?? "", peer ?? ""] as const,
        ),
      }),
    );
  } else if (command === "leave") {
    break;
  }
}
await page.evaluate(() => {
  window.aspenCall?.leave();
});
await browser.close();
process.exit(0);
