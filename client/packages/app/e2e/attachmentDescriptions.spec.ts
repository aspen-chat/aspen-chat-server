import { expect, test, type Page } from "@playwright/test";
import { general, me, settleAnimations, signInToWorld } from "./world";

/**
 * Describing a picture before sending it, against the stubbed world: the description reaches
 * the server with the attachment, before the message that carries it, and readers find it as
 * the picture's text alternative and beneath it in the gallery.
 */

/** A one-pixel PNG, decodable, so the app measures it and the browser draws it. */
const PIXEL =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

const attachmentId = "0190f0a0-0000-7000-8002-000000000001";

interface Recorded {
  /** Each request the attachment routes and posting answered, in order. */
  calls: string[];
  descriptions: (string | null)[];
}

/** Answers the upload, the description, and the message, recording what each was asked. */
async function withUploads(page: Page, recorded: Recorded) {
  let description: string | null = null;
  const record = () => ({
    id: attachmentId,
    fileName: "cat.png",
    mimeType: "image/png",
    downloadUrl: `data:image/png;base64,${PIXEL}`,
    // Drawn larger than its one pixel, as a photo would be, so it is a picture a finger can tap.
    width: 200,
    height: 150,
    ...(description === null ? {} : { description }),
  });
  await page.route(/\/api\/v1\/(attachments|uploads|channels\/[^/]+\/messages)/, (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace("/api/v1", "");
    const method = request.method();
    if (method === "POST" && path === "/attachments") {
      recorded.calls.push("reserve");
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
      recorded.calls.push("confirm");
      return route.fulfill({ json: record() });
    }
    if (method === "PATCH" && path === `/attachments/${attachmentId}`) {
      const body = request.postDataJSON() as { description: string | null };
      description = body.description;
      recorded.calls.push("describe");
      recorded.descriptions.push(body.description);
      return route.fulfill({ json: record() });
    }
    if (method === "POST" && path === `/channels/${general}/messages`) {
      const body = request.postDataJSON() as { content: string; attachments: string[] };
      recorded.calls.push("send");
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
          content: body.content,
          attachments: body.attachments,
          mentions: { users: [], roles: [], everyone: false },
        },
      });
    }
    return route.fallback();
  });
}

test("a picture described before sending carries its description to readers", async ({ page }) => {
  const recorded: Recorded = { calls: [], descriptions: [] };
  await signInToWorld(page, (p) => withUploads(p, recorded));
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await page.locator("form input[type=file]").setInputFiles({
    name: "cat.png",
    mimeType: "image/png",
    buffer: Buffer.from(PIXEL, "base64"),
  });
  const files = page.getByRole("list", { name: "Files to send" });
  await expect(files).toBeVisible();
  await expect.poll(() => recorded.calls).toContain("confirm");

  await files.getByRole("button", { name: "Describe cat.png" }).click();
  const dialog = page.getByRole("dialog", { name: "Describe cat.png" });
  await dialog
    .getByRole("textbox", { name: "Description" })
    .fill("  A grey cat asleep on a keyboard.  ");
  await dialog.getByRole("button", { name: "Save" }).click();
  await expect(dialog).toBeHidden();
  await expect.poll(() => recorded.descriptions).toEqual(["A grey cat asleep on a keyboard."]);
  // Described now, the control offers to change it.
  await expect(
    files.getByRole("button", { name: "Edit the description of cat.png" }),
  ).toBeVisible();

  await page.getByRole("textbox", { name: "Message" }).fill("Look");
  await page.getByRole("button", { name: "Send", exact: true }).click();
  await expect.poll(() => recorded.calls.at(-1)).toBe("send");
  expect(recorded.calls.indexOf("describe")).toBeLessThan(recorded.calls.indexOf("send"));

  const picture = page.getByRole("img", { name: "A grey cat asleep on a keyboard." });
  await expect(picture).toBeVisible();
  await settleAnimations(page);
  await page.getByRole("button", { name: "Open image" }).last().click();
  const gallery = page.getByRole("dialog", { name: "Images" });
  await expect(gallery.getByText("A grey cat asleep on a keyboard.")).toBeVisible();
});

test("a description given while the picture uploads reaches the server once it is there", async ({
  page,
}) => {
  const recorded: Recorded = { calls: [], descriptions: [] };
  let finishUpload: () => void = () => undefined;
  const uploaded = new Promise<void>((resolve) => {
    finishUpload = resolve;
  });
  await signInToWorld(page, async (p) => {
    await withUploads(p, recorded);
    // Held until the description is saved, so the picture is still uploading then.
    await p.route(/\/api\/v1\/uploads\/attachment$/, async (route) => {
      await uploaded;
      await route.fulfill({ status: 200, body: "" });
    });
  });
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await page.locator("form input[type=file]").setInputFiles({
    name: "cat.png",
    mimeType: "image/png",
    buffer: Buffer.from(PIXEL, "base64"),
  });
  const files = page.getByRole("list", { name: "Files to send" });
  await expect(files.getByRole("progressbar")).toBeVisible();
  await files.getByRole("button", { name: "Describe cat.png" }).click();
  const dialog = page.getByRole("dialog", { name: "Describe cat.png" });
  await dialog.getByRole("textbox", { name: "Description" }).fill("A sleepy cat");
  await dialog.getByRole("button", { name: "Save" }).click();
  expect(recorded.descriptions).toEqual([]);

  finishUpload();
  await expect.poll(() => recorded.descriptions).toEqual(["A sleepy cat"]);
  expect(recorded.calls).toEqual(["reserve", "confirm", "describe"]);
});
