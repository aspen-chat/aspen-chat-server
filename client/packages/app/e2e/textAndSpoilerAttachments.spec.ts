import { expect, test, type Page } from "@playwright/test";
import { general, me, settleAnimations, signInToWorld } from "./world";

/**
 * Text and code files shown in their messages, and attachments marked spoilers, against the
 * stubbed world: a file's first twenty lines show, with a download for the rest, and a spoiler
 * stays covered, in the message and in the gallery, until the reader reveals it.
 */

/** A one-pixel PNG, decodable, so the app measures it and the browser draws it. */
const PIXEL =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/** A tenth of a second of silence as 8 kHz, 8-bit mono PCM in a WAV file. */
function silence(): Buffer {
  const samples = 800;
  const wav = Buffer.alloc(44 + samples, 128);
  wav.write("RIFF", 0, "ascii");
  wav.writeUInt32LE(36 + samples, 4);
  wav.write("WAVEfmt ", 8, "ascii");
  wav.writeUInt32LE(16, 16);
  wav.writeUInt16LE(1, 20);
  wav.writeUInt16LE(1, 22);
  wav.writeUInt32LE(8000, 24);
  wav.writeUInt32LE(8000, 28);
  wav.writeUInt16LE(1, 32);
  wav.writeUInt16LE(8, 34);
  wav.write("data", 36, "ascii");
  wav.writeUInt32LE(samples, 40);
  return wav;
}

interface Stored {
  id: string;
  fileName: string;
  mimeType: string;
  spoiler: boolean;
  bytes: Buffer;
}

/** What the stub was asked to change on each attachment, in order. */
type Changes = { id: string; spoiler?: boolean; description?: string | null }[];

/**
 * Answers uploads, changes, and posting, keeping each file's bytes for storage to serve back,
 * as the deployment's read path does, to any origin.
 */
async function withUploads(page: Page, changes: Changes) {
  const files = new Map<string, Stored>();
  let next = 1;
  const record = (file: Stored) => ({
    id: file.id,
    fileName: file.fileName,
    mimeType: file.mimeType,
    downloadUrl: `https://media.test/${file.id}`,
    ...(file.mimeType.startsWith("image/") ? { width: 200, height: 150 } : {}),
    spoiler: file.spoiler,
  });
  await page.route("https://media.test/*", (route) => {
    const id = new URL(route.request().url()).pathname.slice(1);
    const file = files.get(id);
    return file === undefined
      ? route.fulfill({ status: 404 })
      : route.fulfill({
          contentType: file.mimeType,
          headers: { "access-control-allow-origin": "*" },
          body: file.bytes,
        });
  });
  await page.route(/\/api\/v1\/(attachments|uploads|channels\/[^/]+\/messages)/, (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.replace("/api/v1", "");
    const method = request.method();
    if (method === "POST" && path === "/attachments") {
      const body = request.postDataJSON() as { fileName: string; mimeType: string };
      const id = `0190f0a0-0000-7000-8003-${String(next++).padStart(12, "0")}`;
      files.set(id, { id, ...body, spoiler: false, bytes: Buffer.alloc(0) });
      return route.fulfill({
        status: 201,
        json: {
          id,
          uploadUrl: `${url.origin}/api/v1/uploads/${id}`,
          expiresAt: new Date(Date.now() + 600_000).toISOString(),
        },
      });
    }
    const upload = /^\/uploads\/(.+)$/.exec(path);
    if (method === "PUT" && upload?.[1] !== undefined) {
      const file = files.get(upload[1]);
      if (file !== undefined) {
        file.bytes = request.postDataBuffer() ?? Buffer.alloc(0);
      }
      return route.fulfill({ status: 200, body: "" });
    }
    const confirm = /^\/attachments\/(.+)\/confirm$/.exec(path);
    const confirmed = confirm?.[1] === undefined ? undefined : files.get(confirm[1]);
    if (method === "POST" && confirmed !== undefined) {
      return route.fulfill({ json: record(confirmed) });
    }
    const change = /^\/attachments\/([^/]+)$/.exec(path);
    const changed = change?.[1] === undefined ? undefined : files.get(change[1]);
    if (method === "PATCH" && changed !== undefined) {
      const body = request.postDataJSON() as { spoiler?: boolean; description?: string | null };
      changes.push({ id: changed.id, ...body });
      if (body.spoiler !== undefined) {
        changed.spoiler = body.spoiler;
      }
      return route.fulfill({ json: record(changed) });
    }
    if (method === "POST" && path === `/channels/${general}/messages`) {
      const body = request.postDataJSON() as { content: string; attachments: string[] };
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
          content: body.content,
          attachments: body.attachments,
          mentions: { users: [], roles: [], everyone: false },
        },
      });
    }
    return route.fallback();
  });
}

async function openGeneral(page: Page) {
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
}

/** Sends what the box holds once every file in it has uploaded. */
async function send(page: Page, text: string) {
  const files = page.getByRole("list", { name: "Files to send" });
  await expect(files.getByRole("progressbar")).toHaveCount(0);
  await page.getByRole("textbox", { name: "Message" }).fill(text);
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect(files).toBeHidden();
}

const lines = (count: number, word: string) =>
  Array.from({ length: count }, (_, i) => `${word} ${String(i + 1)}`).join("\n") + "\n";

test("a code file shows its first twenty lines and offers the rest as a download", async ({
  page,
}) => {
  await signInToWorld(page, (p) => withUploads(p, []));
  await openGeneral(page);
  await page.locator("form input[type=file]").setInputFiles([
    // No type, as most systems give a source file.
    { name: "main.rs", mimeType: "", buffer: Buffer.from(lines(30, "let line")) },
    { name: "notes.txt", mimeType: "text/plain", buffer: Buffer.from(lines(3, "note")) },
  ]);
  await send(page, "Files");

  const code = page.getByRole("region", { name: "Contents of main.rs" });
  await expect(code).toContainText("let line 20");
  await expect(code).not.toContainText("let line 21");
  const notes = page.getByRole("region", { name: "Contents of notes.txt" });
  await expect(notes).toContainText("note 3");
  // One file has more than it shows.
  const more = page.getByRole("button", { name: "Download to see more" });
  await expect(more).toHaveCount(1);
  const download = page.waitForEvent("download");
  await more.click();
  expect((await download).suggestedFilename()).toBe("main.rs");
});

test("an attachment marked a spoiler is covered until the reader reveals it", async ({ page }) => {
  const changes: Changes = [];
  await signInToWorld(page, (p) => withUploads(p, changes));
  await openGeneral(page);
  await page.locator("form input[type=file]").setInputFiles([
    { name: "cat.png", mimeType: "image/png", buffer: Buffer.from(PIXEL, "base64") },
    { name: "ending.txt", mimeType: "text/plain", buffer: Buffer.from("They all lived.\n") },
    { name: "twist.wav", mimeType: "audio/wav", buffer: silence() },
  ]);
  const files = page.getByRole("list", { name: "Files to send" });
  await expect(files.getByRole("progressbar")).toHaveCount(0);
  const mark = files.getByRole("button", { name: "Mark cat.png as a spoiler" });
  await mark.click();
  await expect(mark).toHaveAttribute("aria-pressed", "true");
  await files.getByRole("button", { name: "Mark ending.txt as a spoiler" }).click();
  await files.getByRole("button", { name: "Mark twist.wav as a spoiler" }).click();
  await expect.poll(() => changes.map((c) => c.spoiler)).toEqual([true, true, true]);
  await send(page, "Spoilers");

  const message = page.getByRole("list", { name: "Attachments" }).last();
  // Covered, each is inert, only its Reveal spoiler button to assistive technology.
  await expect(message.getByRole("button", { name: "Reveal spoiler" })).toHaveCount(3);
  const inert = message.locator("[inert]");
  await expect(inert.getByRole("group", { name: "twist.wav", exact: true })).toHaveCount(1);
  await expect(inert.getByRole("img", { name: "Image: cat.png" })).toHaveCount(1);
  await expect(inert.getByRole("region", { name: "Contents of ending.txt" })).toHaveCount(1);

  await settleAnimations(page);
  // Sounds show first, then text and code, then pictures.
  await message.getByRole("button", { name: "Reveal spoiler" }).first().click();
  await expect(message.getByRole("button", { name: "Play twist.wav" })).toBeVisible();
  await expect(message.getByRole("button", { name: "Reveal spoiler" })).toHaveCount(2);
  await message.getByRole("button", { name: "Reveal spoiler" }).first().click();
  await expect(message.getByRole("region", { name: "Contents of ending.txt" })).toContainText(
    "They all lived.",
  );
  await expect(message.getByRole("button", { name: "Reveal spoiler" })).toHaveCount(1);
  await message.getByRole("button", { name: "Reveal spoiler" }).click();
  await expect(message.getByRole("img", { name: "Image: cat.png" })).toBeVisible();
  await expect(inert).toHaveCount(0);
});
