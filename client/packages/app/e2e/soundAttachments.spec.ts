import { expect, test, type Page } from "@playwright/test";
import { general, me, signInToWorld } from "./world";

/**
 * Sounds attached to messages, against the stubbed world: a sound plays in its own small player,
 * which plays and pauses, shows the time played against its length, scrubs, and turns down and
 * mutes, and a sound the browser cannot play is offered for download instead.
 */

const attachmentId = "0190f0a0-0000-7000-8002-000000000101";
const SOUND_URL = "https://media.test/tone.wav";
// Not a whole number of tenths, so the position must still reach its end when the sound does.
const SOUND_SECONDS = 3.04;

/** `seconds` of a quiet 440 Hz tone as 8 kHz, 8-bit mono PCM in a WAV file. */
function tone(seconds: number): Buffer {
  const rate = 8000;
  const samples = Math.round(rate * seconds);
  const wav = Buffer.alloc(44 + samples);
  wav.write("RIFF", 0, "ascii");
  wav.writeUInt32LE(36 + samples, 4);
  wav.write("WAVEfmt ", 8, "ascii");
  wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20);
  wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(rate, 24);
  wav.writeUInt32LE(rate, 28);
  wav.writeUInt16LE(1, 32);
  wav.writeUInt16LE(8, 34);
  wav.write("data", 36, "ascii");
  wav.writeUInt32LE(samples, 40);
  for (let i = 0; i < samples; i += 1) {
    wav.writeUInt8(128 + Math.round(8 * Math.sin((2 * Math.PI * 440 * i) / rate)), 44 + i);
  }
  return wav;
}

/** Answers the upload and the message for one sound, whose bytes storage serves as `body`. */
async function withSound(page: Page, mimeType: string, body: Buffer) {
  const record = {
    id: attachmentId,
    fileName: "tone.wav",
    mimeType,
    downloadUrl: SOUND_URL,
  };
  // Served in ranges, as storage serves it: a browser seeks only in a file it can fetch from
  // any byte.
  await page.route(SOUND_URL, (route) => {
    const range = /^bytes=(\d+)-(\d*)$/.exec(route.request().headers().range ?? "");
    if (range === null) {
      return route.fulfill({ contentType: mimeType, headers: { "Accept-Ranges": "bytes" }, body });
    }
    const start = Number(range[1]);
    const end = range[2] === "" ? body.length - 1 : Math.min(Number(range[2]), body.length - 1);
    return route.fulfill({
      status: 206,
      contentType: mimeType,
      headers: {
        "Accept-Ranges": "bytes",
        "Content-Range": `bytes ${String(start)}-${String(end)}/${String(body.length)}`,
      },
      body: body.subarray(start, end + 1),
    });
  });
  await page.route(/\/api\/v1\/(attachments|uploads|channels\/[^/]+\/messages)/, (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace("/api/v1", "");
    const method = request.method();
    if (method === "POST" && path === "/attachments") {
      return route.fulfill({
        status: 201,
        json: {
          id: attachmentId,
          uploadUrl: `${new URL(request.url()).origin}/api/v1/uploads/attachment`,
          expiresAt: new Date(Date.now() + 600_000).toISOString(),
        },
      });
    }
    if (method === "PUT" && path === "/uploads/attachment") {
      return route.fulfill({ status: 200, body: "" });
    }
    if (method === "POST" && path === `/attachments/${attachmentId}/confirm`) {
      return route.fulfill({ json: record });
    }
    if (method === "GET" && path === `/attachments/${attachmentId}`) {
      return route.fulfill({ json: record });
    }
    if (method === "POST" && path === `/channels/${general}/messages`) {
      const sent = request.postDataJSON() as { content: string; attachments: string[] };
      return route.fulfill({
        status: 201,
        json: {
          id: "0190f0a0-0000-7000-8001-000000000996",
          channelId: general,
          author: me,
          timestamp: new Date().toISOString(),
          editedAt: null,
          linkPreviews: [],
          linkedMessages: [],
          alteredBy: [],
          kind: "standard",
          poll: null,
          thread: null,
          echoOf: null,
          content: sent.content,
          attachments: sent.attachments,
          mentions: { users: [], roles: [], everyone: false },
        },
      });
    }
    return route.fallback();
  });
}

async function sendSound(page: Page, mimeType: string, body: Buffer) {
  await signInToWorld(page, (p) => withSound(p, mimeType, body));
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await page.locator("form input[type=file]").setInputFiles({
    name: "tone.wav",
    mimeType,
    buffer: body,
  });
  await expect(page.getByRole("list", { name: "Files to send" })).toBeVisible();
  await page.getByRole("textbox", { name: "Message" }).fill("Listen");
  await page.getByRole("button", { name: "Send", exact: true }).click();
}

test("a sound plays, pauses, and scrubs in its own player", async ({ page }) => {
  await sendSound(page, "audio/wav", tone(SOUND_SECONDS));
  const player = page.getByRole("group", { name: "tone.wav", exact: true });
  await expect(player).toBeVisible();
  await expect(player.getByRole("link", { name: "tone.wav" })).toHaveAttribute(
    "download",
    "tone.wav",
  );
  // Its length is known from the start of the file, before anything plays.
  await expect(player.getByText("0:00 / 0:03")).toBeVisible();
  const position = player.getByRole("slider", { name: "Position in tone.wav" });
  await expect(position).toHaveAttribute("aria-valuetext", "0:00 of 0:03");

  await player.getByRole("button", { name: "Play tone.wav" }).click();
  const pause = player.getByRole("button", { name: "Pause tone.wav" });
  await expect(pause).toBeVisible();
  await expect(player.getByText("0:01 / 0:03")).toBeVisible();
  await pause.click();
  await expect(player.getByRole("button", { name: "Play tone.wav" })).toBeVisible();

  // Under ten seconds, each arrow key moves it a tenth of a second; End goes to the end.
  await position.focus();
  await page.keyboard.press("Home");
  await expect(position).toHaveAttribute("aria-valuetext", "0:00 of 0:03");
  await page.keyboard.press("End");
  await expect(position).toHaveAttribute("aria-valuetext", "0:03 of 0:03");
  await expect
    .poll(() => player.locator("audio").evaluate((audio: HTMLAudioElement) => audio.currentTime))
    .toBeCloseTo(SOUND_SECONDS, 1);
  await page.keyboard.press("Home");
  for (let i = 0; i < 15; i += 1) {
    await page.keyboard.press("ArrowRight");
  }
  await expect(position).toHaveAttribute("aria-valuetext", "0:01 of 0:03");
  await expect
    .poll(() => player.locator("audio").evaluate((audio: HTMLAudioElement) => audio.currentTime))
    .toBeCloseTo(1.5, 1);

  // Played to its end, its position sits at the end and it offers to play it again.
  await player.getByRole("button", { name: "Play tone.wav" }).click();
  const replay = player.getByRole("button", { name: "Replay tone.wav" });
  await expect(replay).toBeVisible();
  await expect(position).toHaveAttribute("aria-valuetext", "0:03 of 0:03");
  await expect(position).toHaveValue((await position.getAttribute("max")) ?? "");
  await replay.click();
  await expect(player.getByRole("button", { name: "Pause tone.wav" })).toBeVisible();
  await expect(player.getByText("0:01 / 0:03")).toBeVisible();
});

test("a sound's volume turns down and mutes", async ({ page }) => {
  await sendSound(page, "audio/wav", tone(SOUND_SECONDS));
  const player = page.getByRole("group", { name: "tone.wav", exact: true });
  const element = player.locator("audio");
  const volume = player.getByRole("slider", { name: "Volume of tone.wav" });
  await expect(volume).toHaveAttribute("aria-valuetext", "100%");

  // Each arrow key moves it five percent.
  await volume.focus();
  for (let i = 0; i < 10; i += 1) {
    await page.keyboard.press("ArrowLeft");
  }
  await expect(volume).toHaveAttribute("aria-valuetext", "50%");
  await expect.poll(() => element.evaluate((audio: HTMLAudioElement) => audio.volume)).toBe(0.5);

  await player.getByRole("button", { name: "Mute tone.wav" }).click();
  await expect(volume).toHaveAttribute("aria-valuetext", "0%");
  expect(await element.evaluate((audio: HTMLAudioElement) => audio.muted)).toBe(true);

  // Unmuted, it comes back at the volume it had.
  await player.getByRole("button", { name: "Unmute tone.wav" }).click();
  await expect(volume).toHaveAttribute("aria-valuetext", "50%");
  expect(await element.evaluate((audio: HTMLAudioElement) => [audio.muted, audio.volume])).toEqual([
    false,
    0.5,
  ]);

  // Turned all the way down, it shows as muted, and unmuting brings it back to full.
  await volume.focus();
  await page.keyboard.press("Home");
  await player.getByRole("button", { name: "Unmute tone.wav" }).click();
  await expect(volume).toHaveAttribute("aria-valuetext", "100%");
});

test("a sound the browser cannot play is offered for download", async ({ page }) => {
  await sendSound(page, "audio/ogg", Buffer.from("not a sound at all"));
  await expect(page.getByRole("link", { name: "tone.wav" })).toBeVisible();
  await expect(page.getByRole("group", { name: "tone.wav", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Play tone.wav" })).toHaveCount(0);
});
