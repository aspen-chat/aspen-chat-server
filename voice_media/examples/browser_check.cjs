// Drives `cargo run -p voice_media --example browser -- <address> 1`: two pages speak (a
// browser's fake microphone), one listens, and the server's single audio slot is made to move
// from one speaker to the other by pausing each in turn. Prints what the listener's browser
// reports of the slot in each phase.
//
// NODE_PATH=<a Playwright install>/node_modules node browser_check.cjs [listener: chromium|firefox]
const { chromium, firefox } = require("playwright");

const url = "http://127.0.0.1:8710";
const launch = {
  chromium: () =>
    chromium.launch({
      args: [
        "--use-fake-device-for-media-stream",
        "--use-fake-ui-for-media-stream",
        "--autoplay-policy=no-user-gesture-required",
      ],
    }),
  firefox: () =>
    firefox.launch({
      firefoxUserPrefs: {
        "media.navigator.streams.fake": true,
        "media.navigator.permission.disabled": true,
        "media.autoplay.default": 0,
      },
    }),
};

async function open(browser, role) {
  const context = await browser.newContext(
    browser.browserType().name() === "chromium" ? { permissions: ["microphone"] } : {},
  );
  const page = await context.newPage();
  page.on("pageerror", (e) => console.error(`${role}: ${e.message}`));
  await page.goto(`${url}/?role=${role}`);
  await page.waitForFunction(() => typeof me !== "undefined" && me !== undefined);
  return page;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function pause(page, paused) {
  const id = await page.evaluate(() => me.id);
  await fetch(`${url}/pause/${id}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ source: "microphone", paused }),
  });
}

(async () => {
  const listenerKind = process.argv[2] ?? "chromium";
  const speakers = await launch.chromium();
  const listenerBrowser = await launch[listenerKind]();
  const ann = await open(speakers, "speak");
  const bob = await open(speakers, "speak");
  const listener = await open(listenerBrowser, "listen");
  const names = {
    [await ann.evaluate(() => me.user)]: "Ann",
    [await bob.evaluate(() => me.user)]: "Bob",
  };
  let last;
  const phase = async (name) => {
    const now = await listener.evaluate(() => window.slotStats());
    const slot = now.slots[0];
    const before = last?.slots[0];
    const d = (k) => (slot?.[k] ?? 0) - (before?.[k] ?? 0);
    console.log(
      JSON.stringify({
        phase: name,
        connection: now.connection,
        carries: names[slot?.user] ?? slot?.user,
        ssrc: slot?.ssrc,
        csrc: slot?.csrc,
        packets: d("packets"),
        lost: d("lost"),
        samples: d("samples"),
        concealed: d("concealed"),
        audioLevel: slot?.audioLevel,
      }),
    );
    last = now;
  };
  await sleep(4000);
  await phase("both speak");
  for (const [name, quiet, loud] of [
    ["Ann paused", ann, bob],
    ["Bob paused", bob, ann],
    ["Ann paused again", ann, bob],
  ]) {
    await pause(loud, false);
    await pause(quiet, true);
    await sleep(3000);
    await phase(name);
  }
  await speakers.close();
  await listenerBrowser.close();
  process.exit(0);
})();
