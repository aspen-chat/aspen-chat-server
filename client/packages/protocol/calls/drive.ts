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
 * a browser synchronising its sound with it matches by holding the sound back too: one that
 * does not leaves them that far apart. A browser first estimates sync on the sender reports,
 * which come every few seconds, and then closes the gap tens of milliseconds a second, so each
 * stream is read for `SYNC_SECONDS` and judged by the median of its last `SETTLED_READINGS`.
 * Under the hold a browser settles with the sound tens of milliseconds ahead of the picture
 * whatever sends it, so the call's streams are judged against a stream the same browser
 * receives at the same time straight from itself, settled within `SYNC_MARGIN_MS` of it. Where
 * each settles wanders by about ten milliseconds either way from one measurement to the next,
 * and the call and the reference differed by up to 23 ms on a machine doing nothing else.
 */
const SYNC_DELAY_MS = 400;
const SYNC_SECONDS = 45;
const SETTLED_READINGS = 20;
const SYNC_MARGIN_MS = 30;

/** How long each person's hearing of the other is measured for. */
const HEARING_MS = 4000;
/** Opus decodes at 48 kHz whatever was captured, and a browser counts samples per channel. */
const OPUS_SAMPLES_PER_MS = 48;

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

function sync(page: Page, streams: readonly string[]): Promise<SyncReport[]> {
  return page.evaluate(
    async ([s, delay, seconds]) => {
      const reported = await window.aspenCall?.sync(s, delay, seconds);
      if (reported === undefined) {
        throw new Error("no call");
      }
      return reported;
    },
    [streams, SYNC_DELAY_MS, SYNC_SECONDS] as const,
  );
}

/** `sync`, and the page's own reference measured over the same time. */
async function syncWithReference(
  page: Page,
  streams: readonly string[],
): Promise<{ reports: SyncReport[]; reference: SyncReport }> {
  const reference = page.evaluate(
    async ([delay, seconds]) => {
      const reported = await window.aspenCall?.reference(delay, seconds);
      if (reported === undefined) {
        throw new Error("no page");
      }
      return reported;
    },
    [SYNC_DELAY_MS, SYNC_SECONDS] as const,
  );
  const [reports, referenced] = await Promise.all([sync(page, streams), reference]);
  return { reports, reference: referenced };
}

/** The median of a report's last `SETTLED_READINGS` offsets, `null` unless every one is read. */
function settled(report: SyncReport): number | null {
  const last = report.offsets.slice(-SETTLED_READINGS);
  if (last.length < SETTLED_READINGS || last.some((offset) => offset === null)) {
    return null;
  }
  const sorted = (last as number[]).toSorted((a, b) => a - b);
  const half = Math.floor(sorted.length / 2);
  const upper = sorted[half] ?? 0;
  return sorted.length % 2 === 1 ? upper : ((sorted[half - 1] ?? 0) + upper) / 2;
}

function inStep(report: SyncReport, reference: SyncReport): boolean {
  const own = settled(report);
  const browsers = settled(reference);
  return own !== null && browsers !== null && Math.abs(own - browsers) <= SYNC_MARGIN_MS;
}

/** What a sync report says against its reference, short enough to print. */
function syncDetail(report: SyncReport, reference: SyncReport) {
  const measured = report.offsets.filter((offset) => offset !== null);
  return {
    settled: settled(report),
    reference: { settled: settled(reference), last: reference.offsets.slice(-8) },
    ...report,
    offsets: measured.slice(-8),
    measured: measured.length,
  };
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

type BrowserName = "chromium" | "firefox";

/** Ann in Chromium and Bob in `bobBrowser`, each in a browser of their own, both in the call. */
async function inCall(
  bobBrowser: BrowserName,
  body: (ann: Page, bob: Page) => Promise<void>,
): Promise<void> {
  const annBrowser = await chromium.launch(BROWSERS.chromium.launch);
  const bobs = BROWSERS[bobBrowser];
  const bobBrowserLaunched = await bobs.type.launch(bobs.launch);
  try {
    await body(
      await open(annBrowser, args.ann, "Ann"),
      await open(bobBrowserLaunched, args.bob, "Bob"),
    );
  } finally {
    await annBrowser.close();
    await bobBrowserLaunched.close();
  }
}

const connected = (r: CallReport) => r.status === "connected" && r.connection === "connected";

/** Whether at least two received pictures decoded more than ten frames since `before`. */
const decoding = (r: CallReport, before: CallReport) =>
  received(r, "video").filter((s) => framesSince(s, before) > 10).length >= 2;

/**
 * Has Ann stop her camera and her screen and start both again, once Bob has seen each stop: what
 * Bob received while both were stopped, and once he decodes both again.
 */
async function restartVideo(
  ann: Page,
  bob: Page,
): Promise<{ idle: CallReport; again: CallReport }> {
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
  const again = await until(
    bob,
    (r) =>
      r.cameras.includes(args.ann.userId) &&
      r.screens.includes(args.ann.userId) &&
      decoding(r, idle),
  );
  return { idle, again };
}

async function scenario(bobBrowser: BrowserName): Promise<void> {
  console.log(`Ann in Chromium, Bob in ${bobBrowser}`);
  await inCall(bobBrowser, async (ann, bob) => {
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

    // Each hears the other's microphone on an audio slot, decoded with little made up, measured
    // over `HEARING_MS` from when sound first arrives at both, which can be a second or more
    // after both connect on a busy machine; how long it took is in the detail. What was heard is
    // judged by the samples played, not by packets, whose length the sender chooses: Chromium
    // sends 20 ms of sound in each on one machine and 40 ms on a CI runner.
    const hearing = (r: CallReport) => received(r, "audio").some((s) => s.packets > 0);
    const waited = Date.now();
    await Promise.all([until(ann, hearing), until(bob, hearing)]);
    const startedAfterMs = Date.now() - waited;
    const [annBefore, bobBefore] = await Promise.all([report(ann), report(bob)]);
    await sleep(HEARING_MS);
    const [annAfter, bobAfter] = await Promise.all([report(ann), report(bob)]);
    const loudness = (page: Page) => page.evaluate(() => window.aspenCall?.loudness() ?? 0);
    const [annLoudness, bobLoudness] = await Promise.all([loudness(ann), loudness(bob)]);
    // The browsers' fake microphones play a tone, which decodes to sound well above silence.
    for (const [who, before, after, peak] of [
      ["Ann hears Bob", annBefore, annAfter, annLoudness],
      ["Bob hears Ann", bobBefore, bobAfter, bobLoudness],
    ] as const) {
      const sound = { ...heard(before, after), peak, startedAfterMs };
      check(
        who,
        sound.packets > 0 &&
          sound.samples > (OPUS_SAMPLES_PER_MS * HEARING_MS * 9) / 10 &&
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
    // A shared screen, with the sender's encoder bounded.
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

    // Sections no longer wanted go idle and are taken over by what is wanted next: a camera and
    // a screen stopped and started again play on sections the call already has, numbered on
    // from where they stopped (a browser drops numbers it has had as replays).
    const { idle, again } = await restartVideo(ann, bob);
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
        twoSecondsLater: received(still, "video"),
        ann: await report(ann).then((r) => ({ sections: r.sections, video: sent(r, "video") })),
      },
    );
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
  });
}

/**
 * Sync, measured in a call of its own once every other check has finished, so nothing else the
 * machine runs moves what the browsers settle on: Ann's camera and screen, each with its sound,
 * on fresh sections and again on sections taken over, both measured at once beside Bob's
 * reference.
 */
async function syncScenario(bobBrowser: BrowserName): Promise<void> {
  console.log(`Sync: Ann in Chromium, Bob in ${bobBrowser}`);
  await inCall(bobBrowser, async (ann, bob) => {
    await Promise.all([until(ann, connected), until(bob, connected)]);
    await ann.evaluate(async () => {
      await window.aspenCall?.startCamera();
      await window.aspenCall?.startScreenShare();
    });
    await bob.evaluate(() => {
      window.aspenCall?.showVideo();
    });
    const start = await report(bob);
    const showing = (r: CallReport) =>
      r.cameras.includes(args.ann.userId) && r.screens.includes(args.ann.userId);
    const arrived = await until(bob, (r) => showing(r) && decoding(r, start));
    check(
      "Bob decodes Ann's camera and screen",
      showing(arrived) && decoding(arrived, start),
      received(arrived, "video"),
    );
    const streams = [`${args.ann.userId}-camera`, `${args.ann.userId}-share`];
    const inSync = async (what: { camera: string; share: string }) => {
      const {
        reports: [camera, share],
        reference,
      } = await syncWithReference(bob, streams);
      if (camera === undefined || share === undefined) {
        throw new Error("a stream measured went unreported");
      }
      check(what.camera, inStep(camera, reference), syncDetail(camera, reference));
      if (share.audioMid !== null) {
        check(what.share, inStep(share, reference), syncDetail(share, reference));
      }
    };
    await inSync({
      camera: "Bob hears Ann's voice in step with her camera",
      share: "and her screen's sound in step with her screen",
    });
    const { idle, again } = await restartVideo(ann, bob);
    check(
      "Bob decodes them again once Ann starts them again",
      decoding(again, idle),
      received(again, "video"),
    );
    await inSync({
      camera: "and her voice is in step with her camera on its new section",
      share: "and her screen's sound with her screen",
    });
    await Promise.all(
      [ann, bob].map((page) =>
        page.evaluate(() => {
          window.aspenCall?.leave();
        }),
      ),
    );
  });
}

const BOBS: readonly BrowserName[] = ["chromium", "firefox"];
for (const bob of BOBS) {
  await scenario(bob);
}
for (const bob of BOBS) {
  await syncScenario(bob);
}
process.exit(failures === 0 ? 0 : 1);
