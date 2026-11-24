import { expect, test, type Page } from "@playwright/test";
import { general, me, signInToWorld } from "./world";

/**
 * SVGs attached to messages, against the stubbed world. Storage serves an SVG to be saved, as
 * `application/octet-stream`, since a browser opening one would run its scripts, and its record
 * says so (`inline: false`); what is shown of it is the server's preview, inline and in the
 * gallery, and until it has one it shows as the text it is, never as unavailable.
 */

const attachmentId = "0190f0a0-0000-7000-8002-000000000201";
const ORIGINAL = "https://media.test/drawing.svg";
const PREVIEW = "https://media.test/drawing-preview.png";

/** A one-pixel PNG, standing in for the server's preview. */
const PIXEL = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

const SVG =
  '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">\n' +
  '  <rect width="40" height="20" fill="teal"/>\n' +
  "</svg>\n";

/**
 * Answers the upload and the message for one SVG, whose record carries `preview` when given,
 * and serves its bytes as storage does, to be saved, and the preview as a picture.
 */
async function withSvg(page: Page, preview: boolean, fetched: string[]) {
  const record = {
    id: attachmentId,
    fileName: "drawing.svg",
    mimeType: "image/svg+xml",
    downloadUrl: ORIGINAL,
    inline: false,
    ...(preview
      ? { preview: { url: PREVIEW, mimeType: "image/webp", width: 40, height: 20 } }
      : {}),
  };
  await page.route(ORIGINAL, (route) => {
    fetched.push(ORIGINAL);
    return route.fulfill({
      contentType: "application/octet-stream",
      headers: {
        "Access-Control-Allow-Origin": "*",
        "Content-Disposition": 'attachment; filename="drawing.svg"',
      },
      body: SVG,
    });
  });
  await page.route(PREVIEW, (route) => {
    fetched.push(PREVIEW);
    return route.fulfill({
      contentType: "image/png",
      headers: { "Access-Control-Allow-Origin": "*" },
      body: PIXEL,
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
    if (
      (method === "POST" && path === `/attachments/${attachmentId}/confirm`) ||
      (method === "GET" && path === `/attachments/${attachmentId}`)
    ) {
      return route.fulfill({ json: record });
    }
    if (method === "POST" && path === `/channels/${general}/messages`) {
      const sent = request.postDataJSON() as { content: string; attachments: string[] };
      return route.fulfill({
        status: 201,
        json: {
          id: "0190f0a0-0000-7000-8001-000000000995",
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

async function sendSvg(page: Page, preview: boolean): Promise<string[]> {
  const fetched: string[] = [];
  await signInToWorld(page, (p) => withSvg(p, preview, fetched));
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await page.locator("form input[type=file]").setInputFiles({
    name: "drawing.svg",
    mimeType: "image/svg+xml",
    buffer: Buffer.from(SVG),
  });
  await expect(page.getByRole("list", { name: "Files to send" })).toBeVisible();
  await page.getByRole("textbox", { name: "Message" }).fill("A drawing");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  return fetched;
}

test("an SVG shows as its preview, inline and in the gallery", async ({ page }) => {
  const fetched = await sendSvg(page, true);
  const picture = page.getByRole("button", { name: "Image: drawing.svg" });
  await expect(picture.locator("img")).toHaveAttribute("src", PREVIEW);
  await expect(picture.locator("img")).toHaveJSProperty("naturalWidth", 1);
  await expect(page.getByText("Attachment unavailable")).toHaveCount(0);

  // Drawn at its own 40 by 20 pixels, under the controls laid over a picture's corner.
  await picture.focus();
  await page.keyboard.press("Enter");
  const gallery = page.getByRole("dialog", { name: "Images" });
  await expect(gallery).toBeVisible();
  await expect(gallery.locator("img").first()).toHaveAttribute("src", PREVIEW);
  // Nothing tried to show the original, which storage serves to be saved.
  expect(fetched).not.toContain(ORIGINAL);
});

test("an SVG without a preview shows as the text it is", async ({ page }) => {
  await sendSvg(page, false);
  await expect(page.getByText('<rect width="40" height="20" fill="teal"/>')).toBeVisible();
  await expect(page.getByRole("button", { name: "Image: drawing.svg" })).toHaveCount(0);
  await expect(page.getByText("Attachment unavailable")).toHaveCount(0);
});
