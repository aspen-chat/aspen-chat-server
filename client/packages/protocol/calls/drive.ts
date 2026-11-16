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
import type { CallReport, StreamReport, SyncReport } from "./page";

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

/**
 * Sync is checked by holding a stream's picture back by `SYNC_DELAY_MS` at the receiver, which
 * a browser synchronising its sound with it matches by holding the sound back too, until they
 * play within `SYNC_TOLERANCE_MS` of each other: the edge of what people notice as lip sync
 * (ITU-R BT.1359 puts it at 45 ms of sound ahead). A browser first estimates sync on the sender
 * reports, which come every few seconds, and then closes the gap tens of milliseconds a second.
 */
const SYNC_DELAY_MS = 400;
const SYNC_TOLERANCE_MS = 45;
const SYNC_SECONDS = 45;

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

function sync(page: Page, stream: string): Promise<SyncReport> {
  return page.evaluate(
    async ([s, delay, tolerance, seconds]) => {
      const reported = await window.aspenCall?.sync(s, delay, tolerance, seconds);
      if (reported === undefined) {
        throw new Error("no call");
      }
      return reported;
    },
    [stream, SYNC_DELAY_MS, SYNC_TOLERANCE_MS, SYNC_SECONDS] as const,
  );
}

function inStep(report: SyncReport): boolean {
  const last = report.offsets.at(-1);
  return last !== null && last !== undefined && Math.abs(last) <= SYNC_TOLERANCE_MS;
}

/** What a sync report says, short enough to print: the sections and the last offsets seen. */
function syncDetail(report: SyncReport) {
  const measured = report.offsets.filter((offset) => offset !== null);
  return { ...report, offsets: measured.slice(-8), measured: measured.length };
}

/**
 * The frames a received video stream decoded since `before`: all it counts where it has counted
 * fewer than then, since a browser may count a section that resumes from zero (Firefox does).
 */
function framesSince(stream: StreamReport, before: CallReport): number {
  const earlier = received(before, "video").find((b) => b.mid === stream.mid);
  const from =
    earlier === undefined || earlier.framesDecoded > stream.framesDecoded
      ? 0
      : earlier.framesDecoded;
  return stream.framesDecoded - from;
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
    // Without a sound server a browser may hear and record nothing however well the call works.
    for (const [who, page] of [
      ["Ann", ann],
      ["Bob", bob],
    ] as const) {
      const output = await page.evaluate(() => window.aspenCall?.soundOutput());
      check(`${who}'s browser plays sound`, output === "running", {
        state: output,
        fix: "run a sound server (PulseAudio, or PipeWire's) where the browsers run",
      });
    }
    const connected = (r: CallReport) => r.status === "connected" && r.connection === "connected";
    const [annJoined, bobJoined] = await Promise.all([
      until(ann, connected),
      until(bob, connected),
    ]);
    check("both join and connect", connected(annJoined) && connected(bobJoined), {
      ann: annJoined,
      bob: bobJoined,
    });
    // Offers are large and repeat themselves, so signalling is compressed wherever a browser
    // offers to (RFC 7692), which every browser does.
    check(
      "both signal over compressed sockets",
      [annJoined, bobJoined].every((r) => r.signalling?.startsWith("permessage-deflate") === true),
      { ann: annJoined.signalling, bob: bobJoined.signalling },
    );

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
    const cameraStream = `${args.ann.userId}-camera`;
    const lipSync = await sync(bob, cameraStream);
    check("Bob hears Ann's voice in step with her camera", inStep(lipSync), syncDetail(lipSync));

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

    const shareStream = `${args.ann.userId}-share`;
    const shareSync = await sync(bob, shareStream);
    if (shareSync.audioMid !== null) {
      check("and her screen's sound in step with it", inStep(shareSync), syncDetail(shareSync));
    }

    // Sections no longer wanted go idle and are taken over by what is wanted next: a camera and
    // a screen stopped and started again play on sections the call already has, numbered on
    // from where they stopped (a browser drops numbers it has had as replays), and in step.
    await ann.evaluate(() => {
      window.aspenCall?.stopCamera();
    });
    await until(bob, (r) => !r.cameras.includes(args.ann.userId));
    await ann.evaluate(() => {
      window.aspenCall?.stopScreenShare();
    });
    await until(bob, (r) => !r.screens.includes(args.ann.userId));
    const idle = await report(bob);
    await ann.evaluate(async () => {
      await window.aspenCall?.startScreenShare();
      await window.aspenCall?.startCamera();
    });
    const decoding = (r: CallReport, before: CallReport) =>
      received(r, "video").filter((s) => framesSince(s, before) > 10).length >= 2;
    const again = await until(
      bob,
      (r) =>
        r.cameras.includes(args.ann.userId) &&
        r.screens.includes(args.ann.userId) &&
        decoding(r, idle),
    );
    await sleep(2000);
    const still = await report(bob);
    check(
      "a camera and a screen started again play on the sections they left",
      decoding(again, idle) &&
        decoding(still, again) &&
        again.sections.length === idle.sections.length,
      {
        before: { sections: idle.sections, video: received(idle, "video") },
        after: {
          sections: again.sections,
          answered: again.answered,
          video: received(again, "video"),
        },
        ann: await report(ann).then((r) => ({ sections: r.sections, video: sent(r, "video") })),
      },
    );
    const lipSyncAgain = await sync(bob, cameraStream);
    check(
      "and her voice is in step with her camera on its new section",
      inStep(lipSyncAgain),
      syncDetail(lipSyncAgain),
    );
    const shareSyncAgain = await sync(bob, shareStream);
    if (shareSyncAgain.audioMid !== null) {
      check(
        "and her screen's sound with her screen",
        inStep(shareSyncAgain),
        syncDetail(shareSyncAgain),
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
