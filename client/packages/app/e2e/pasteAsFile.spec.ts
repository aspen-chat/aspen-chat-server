import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Pasting text too long for a message, against the stubbed world: it is attached as a text
 * file, the box keeping what it held, while text that fits is left to the browser to paste.
 */

const attachmentId = "0190f0a0-0000-7000-8002-000000000002";

interface Reserved {
  fileName: string;
  mimeType: string;
  byteSize: number;
}

/** Answers the upload, recording what was reserved and the bytes sent to storage. */
async function withUploads(page: Page, reserved: Reserved[], stored: string[]) {
  await page.route(/\/api\/v1\/(attachments|uploads)/, (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace("/api/v1", "");
    const method = request.method();
    if (method === "POST" && path === "/attachments") {
      reserved.push(request.postDataJSON() as Reserved);
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
      stored.push(request.postDataBuffer()?.toString("utf8") ?? "");
      return route.fulfill({ status: 200, body: "" });
    }
    if (method === "POST" && path === `/attachments/${attachmentId}/confirm`) {
      const first = reserved[0];
      return route.fulfill({
        json: {
          id: attachmentId,
          fileName: first?.fileName ?? "",
          mimeType: first?.mimeType ?? "",
          downloadUrl: "https://media.test/message.txt",
        },
      });
    }
    return route.fallback();
  });
}

/**
 * Pastes `text` into the message box as a paste the page did not make, which the browser does
 * not carry out itself; whether the box took it over is what this answers.
 */
async function paste(page: Page, text: string): Promise<boolean> {
  return page.getByRole("textbox", { name: "Message" }).evaluate((box, text) => {
    const data = new DataTransfer();
    data.setData("text/plain", text);
    const event = new ClipboardEvent("paste", { bubbles: true, cancelable: true });
    // Firefox gives a paste made this way an empty copy of `clipboardData` given at its making.
    Object.defineProperty(event, "clipboardData", { value: data });
    box.dispatchEvent(event);
    return event.defaultPrevented;
  }, text);
}

async function openGeneral(page: Page) {
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
}

test("text pasted past the length limit is attached as a text file", async ({ page }) => {
  const reserved: Reserved[] = [];
  const stored: string[] = [];
  await signInToWorld(page, (p) => withUploads(p, reserved, stored));
  await openGeneral(page);
  const box = page.getByRole("textbox", { name: "Message" });
  await box.fill("Here it is:");

  // Written in more than one script, so the file is shown to keep any text as it was.
  const long = "日本語 and English. ".repeat(600);
  expect(await paste(page, long)).toBe(true);

  const files = page.getByRole("list", { name: "Files to send" });
  await expect(files.getByText("message.txt")).toBeVisible();
  await expect(box).toHaveValue("Here it is:");
  await expect.poll(() => stored).toEqual([long]);
  expect(reserved).toEqual([
    {
      fileName: "message.txt",
      mimeType: "text/plain;charset=utf-8",
      byteSize: Buffer.byteLength(long, "utf8"),
    },
  ]);
  await expect(page.getByRole("button", { name: "Send", exact: true })).toBeEnabled();
});

test("text that fits within the length limit is pasted as text", async ({ page }) => {
  const reserved: Reserved[] = [];
  await signInToWorld(page, (p) => withUploads(p, reserved, []));
  await openGeneral(page);
  // Exactly the limit fits.
  expect(await paste(page, "a".repeat(10_000))).toBe(false);
  await expect(page.getByRole("list", { name: "Files to send" })).toBeHidden();
  expect(reserved).toEqual([]);
});

test("a paste that would take what is written past the limit is attached", async ({ page }) => {
  const reserved: Reserved[] = [];
  await signInToWorld(page, (p) => withUploads(p, reserved, []));
  await openGeneral(page);
  const box = page.getByRole("textbox", { name: "Message" });
  await box.fill("a".repeat(9_000));
  expect(await paste(page, "b".repeat(1_001))).toBe(true);
  await expect(page.getByRole("list", { name: "Files to send" })).toBeVisible();

  // Text selected is replaced by the paste, so it is not counted.
  await box.fill("a".repeat(9_000));
  await box.evaluate((element: HTMLTextAreaElement) => {
    element.setSelectionRange(0, 1_000);
  });
  expect(await paste(page, "b".repeat(2_000))).toBe(false);
});
