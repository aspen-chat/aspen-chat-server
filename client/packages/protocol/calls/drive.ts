/**
 * Drives real calls in real browsers through `page.ts`, for `scripts/check_calls.py`: two people
 * join the same call, Ann always in Chromium and Bob in each browser in turn, and each checks
 * what the other's browser received of them. Prints one line per check and exits non-zero when
 * any fails.
 *
 * ASPEN_CALL_CHECK='{"base": …, "channel": …, "ann": <session>, "bob": <session>}' tsx calls/drive.ts
 *
 * The sessions come in the environment rather than as arguments, which would print them.
 */

import { chromium, firefox, type Browser, type BrowserType, type Page } from "@playwright/test";
import type { Session } from "../src";
import type { CallReport, StreamReport } from "./page";

interface Arguments {
  readonly base: string;
  readonly channel: string;
  readonly ann: Session;
  readonly bob: Session;
}

const BROWSERS: Record<"chromium" | "firefox", { type: BrowserType; launch: object }> = {
  chromium: {
    type: chromium,
    launch: {
      args: [
        "--use-fake-device-for-media-stream",
        "--use-fake-ui-for-media-stream",
        "--autoplay-policy=no-user-gesture-required",
      ],
    },
  },
  firefox: {
    type: firefox,
    launch: {
      firefoxUserPrefs: {
        "media.navigator.streams.fake": true,
        "media.navigator.permission.disabled": true,
        "media.autoplay.default": 0,
        // A screen share's picture in a browser that can only fake cameras and microphones.
        "media.getdisplaymedia.screenshare.fake": true,
      },
    },
  },
};

const args = JSON.parse(process.env.ASPEN_CALL_CHECK ?? "{}") as Arguments;
let failures = 0;

function check(what: string, passed: boolean, detail: unknown): void {
  if (passed) {
    console.log(`  ok: ${what}`);
  } else {
    failures += 1;
    console.log(`  FAILED: ${what}: ${JSON.stringify(detail)}`);
  }
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function open(browser: Browser, session: Session, name: string): Promise<Page> {
  const context = await browser.newContext(
    browser.browserType().name() === "chromium" ? { permissions: ["microphone", "camera"] } : {},
  );
  const page = await context.newPage();
  page.on("pageerror", (error) => {
    console.log(`  ${name}'s page: ${error.message}`);
  });
  await page.goto(`${args.base}/`);
  await page.waitForFunction(() => window.aspenCall !== undefined);
  await page.evaluate(
    async ([s, channel]) => {
      await window.aspenCall?.join(s, channel);
    },
    [session, args.channel] as const,
  );
  return page;
}

function report(page: Page): Promise<CallReport> {
  return page.evaluate(async () => {
    const reported = await window.aspenCall?.report();
    if (reported === undefined) {
      throw new Error("no call");
    }
    return reported;
  });
}

/** Waits until `ready` holds of the page's report, up to `seconds`; the last report either way. */
async function until(
  page: Page,
  ready: (report: CallReport) => boolean,
  seconds = 15,
): Promise<CallReport> {
  const deadline = Date.now() + seconds * 1000;
  let last = await report(page);
  while (!ready(last) && Date.now() < deadline) {
    await sleep(250);
    last = await report(page);
  }
  return last;
}

function received(report: CallReport, kind: string): StreamReport[] {
  return report.streams.filter((s) => s.direction === "inbound" && s.kind === kind);
}

function sent(report: CallReport, kind: string): StreamReport[] {
  return report.streams.filter((s) => s.direction === "outbound" && s.kind === kind);
}

/** Received audio between two reports: what arrived, and how much of it was made up. */
function heard(before: CallReport, after: CallReport) {
  const total = (report: CallReport, field: "packets" | "samples" | "concealed") =>
    received(report, "audio").reduce((sum, stream) => sum + stream[field], 0);
  return {
    packets: total(after, "packets") - total(before, "packets"),
    samples: total(after, "samples") - total(before, "samples"),
    concealed: total(after, "concealed") - total(before, "concealed"),
  };
}

async function scenario(bobBrowser: "chromium" | "firefox"): Promise<void> {
  console.log(`Ann in Chromium, Bob in ${bobBrowser}`);
  const annBrowser = await chromium.launch(BROWSERS.chromium.launch);
  const bobs = BROWSERS[bobBrowser];
  const bobBrowserLaunched = await bobs.type.launch(bobs.launch);
  try {
    const ann = await open(annBrowser, args.ann, "Ann");
    const bob = await open(bobBrowserLaunched, args.bob, "Bob");
    const connected = (r: CallReport) => r.status === "connected" && r.connection === "connected";
    const [annJoined, bobJoined] = await Promise.all([
      until(ann, connected),
      until(bob, connected),
    ]);
    check("both join and connect", connected(annJoined) && connected(bobJoined), {
      ann: annJoined,
      bob: bobJoined,
    });

    // Each hears the other's microphone on an audio slot, decoded with little made up.
    const [annBefore, bobBefore] = await Promise.all([report(ann), report(bob)]);
    await sleep(4000);
    const [annAfter, bobAfter] = await Promise.all([report(ann), report(bob)]);
    const loudness = (page: Page) => page.evaluate(() => window.aspenCall?.loudness() ?? 0);
    const [annLoudness, bobLoudness] = await Promise.all([loudness(ann), loudness(bob)]);
    // The browsers' fake microphones play a tone, which decodes to sound well above silence.
    for (const [who, before, after, peak] of [
      ["Ann hears Bob", annBefore, annAfter, annLoudness],
      ["Bob hears Ann", bobBefore, bobAfter, bobLoudness],
    ] as const) {
      const sound = { ...heard(before, after), peak };
      check(
        who,
        sound.packets > 150 &&
          sound.samples > 0 &&
          sound.concealed < sound.samples * 0.1 &&
          sound.peak > 0.01,
        sound,
      );
    }

    // A camera reaches whoever shows video, and only then.
    await ann.evaluate(async () => {
      await window.aspenCall?.startCamera();
    });
    await sleep(1500);
    check(
      "an unwatched camera is not received",
      received(await report(bob), "video").every((s) => s.framesDecoded === 0),
      received(await report(bob), "video"),
    );
    await bob.evaluate(() => {
      window.aspenCall?.showVideo();
    });
    const watching = await until(bob, (r) => r.cameras.includes(args.ann.userId));
    check(
      "Bob shows Ann's camera once he watches",
      watching.cameras.includes(args.ann.userId),
      watching,
    );
    const camera = await until(bob, (r) => received(r, "video").some((s) => s.framesDecoded > 30));
    check(
      "and decodes it",
      received(camera, "video").some((s) => s.framesDecoded > 30),
      received(camera, "video"),
    );

    // A shared screen, with the sender's encoder bounded and the server's start bitrate offered.
    await ann.evaluate(async () => {
      await window.aspenCall?.startScreenShare();
    });
    const sharing = await until(bob, (r) => r.screens.includes(args.ann.userId));
    check("Bob shows Ann's screen", sharing.screens.includes(args.ann.userId), sharing);
    const screen = await until(
      bob,
      (r) => received(r, "video").filter((s) => s.framesDecoded > 10).length >= 2,
    );
    check(
      "and decodes it beside her camera",
      received(screen, "video").filter((s) => s.framesDecoded > 10).length >= 2,
      received(screen, "video"),
    );
    const annSending = await report(ann);
    check(
      "Ann sends her camera and her screen",
      sent(annSending, "video").filter((s) => s.packets > 0).length >= 2,
      sent(annSending, "video"),
    );
    const shareSound = sent(annSending, "audio").find((s) => s.codec.includes("stereo=1"));
    if (shareSound !== undefined) {
      check(
        "her screen's sound goes out in stereo without DTX",
        shareSound.codec.includes("usedtx=0"),
        shareSound,
      );
    }

    // Leaving takes everything of Ann's away from Bob.
    await ann.evaluate(() => {
      window.aspenCall?.leave();
    });
    const left = await until(bob, (r) => r.cameras.length === 0 && r.screens.length === 0);
    check(
      "Ann leaving takes her video away",
      left.cameras.length === 0 && left.screens.length === 0,
      left,
    );
    await bob.evaluate(() => {
      window.aspenCall?.leave();
    });
  } finally {
    await annBrowser.close();
    await bobBrowserLaunched.close();
  }
}

for (const bob of ["chromium", "firefox"] as const) {
  await scenario(bob);
}
process.exit(failures === 0 ? 0 : 1);
