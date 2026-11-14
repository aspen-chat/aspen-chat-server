import { expect, test, type Page } from "@playwright/test";
import { signInToWorld } from "./world";

/**
 * Files pasted into the message box and dropped on its pane, against the stubbed world: each is
 * attached, text beside them is written in, a picture without a name of its own is named for the
 * moment it came, and a picture waiting to be sent can be seen whole, and beside its
 * description, before its upload is done.
 */

/** A 1×1 PNG, which the box measures before it uploads. */
const PNG = Uint8Array.from(
  atob(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  ),
  (c) => c.charCodeAt(0),
);

const TIMESTAMPED_PNG = /^img_\d{8}_\d{6}\.png$/;

interface Reserved {
  fileName: string;
  mimeType: string;
}

/**
 * Answers uploads, recording what was reserved; with `hold`, storage never answers, so every
 * file stays uploading.
 */
async function withUploads(page: Page, reserved: Reserved[], hold = false) {
  await page.route(/\/api\/v1\/(attachments|uploads)/, (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.replace("/api/v1", "");
    const method = request.method();
    if (method === "POST" && path === "/attachments") {
      const index = reserved.push(request.postDataJSON() as Reserved) - 1;
      const id = `0190f0a0-0000-7000-8002-${String(index).padStart(12, "0")}`;
      return route.fulfill({
        status: 201,
        json: {
          id,
          uploadUrl: `${url.origin}/api/v1/uploads/${id}`,
          expiresAt: new Date(Date.now() + 600_000).toISOString(),
        },
      });
    }
    if (method === "PUT" && path.startsWith("/uploads/")) {
      return hold ? undefined : route.fulfill({ status: 200, body: "" });
    }
    const confirmed = /^\/attachments\/([^/]+)\/confirm$/.exec(path);
    if (method === "POST" && confirmed !== null) {
      const id = confirmed[1] ?? "";
      const first = reserved[Number(id.slice(-12))];
      return route.fulfill({
        json: {
          id,
          fileName: first?.fileName ?? "",
          mimeType: first?.mimeType ?? "",
          downloadUrl: `https://media.test/${id}`,
        },
      });
    }
    return route.fallback();
  });
}

interface Carried {
  files: { name: string; type: string; png?: boolean }[];
  text?: string;
}

/**
 * Pastes `carried` into the message box as a paste the page did not make, which the browser
 * does not carry out itself; answers whether the box took it over.
 */
async function paste(page: Page, carried: Carried): Promise<boolean> {
  return page.getByRole("textbox", { name: "Message" }).evaluate(
    (box, { carried, png }) => {
      const data = new DataTransfer();
      for (const file of carried.files) {
        const bytes = file.png === true ? new Uint8Array(png) : new TextEncoder().encode("x");
        data.items.add(new File([bytes], file.name, { type: file.type }));
      }
      if (carried.text !== undefined) {
        data.setData("text/plain", carried.text);
      }
      const event = new ClipboardEvent("paste", { bubbles: true, cancelable: true });
      // Firefox gives a paste made this way an empty copy of `clipboardData` given at its making.
      Object.defineProperty(event, "clipboardData", { value: data });
      box.dispatchEvent(event);
      return event.defaultPrevented;
    },
    { carried, png: Array.from(PNG) },
  );
}

/** Drags `carried` over the channel's messages and drops it there. */
async function drop(page: Page, carried: Carried) {
  const data = await page.evaluateHandle(
    ({ carried, png }) => {
      const data = new DataTransfer();
      for (const file of carried.files) {
        const bytes = file.png === true ? new Uint8Array(png) : new TextEncoder().encode("x");
        data.items.add(new File([bytes], file.name, { type: file.type }));
      }
      if (carried.text !== undefined) {
        data.setData("text/plain", carried.text);
      }
      return data;
    },
    { carried, png: Array.from(PNG) },
  );
  const list = page.locator("[data-message-list]").first();
  await list.dispatchEvent("dragenter", { dataTransfer: data });
  await list.dispatchEvent("dragover", { dataTransfer: data });
  await list.dispatchEvent("drop", { dataTransfer: data });
}

async function openGeneral(page: Page) {
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
}

test("a pasted picture is attached under a name of the moment it came", async ({ page }) => {
  const reserved: Reserved[] = [];
  await signInToWorld(page, (p) => withUploads(p, reserved));
  await openGeneral(page);
  expect(await paste(page, { files: [{ name: "image.png", type: "image/png", png: true }] })).toBe(
    true,
  );
  const files = page.getByRole("list", { name: "Files to send" });
  await expect(files.getByText(TIMESTAMPED_PNG)).toBeVisible();
  await expect(files.locator("img")).toBeVisible();
  await expect.poll(() => reserved.map((r) => r.fileName)).toHaveLength(1);
  expect(reserved[0]?.fileName).toMatch(TIMESTAMPED_PNG);
  expect(reserved[0]?.mimeType).toBe("image/png");
});

test("text pasted beside a picture is written in, and the picture attached", async ({ page }) => {
  const reserved: Reserved[] = [];
  await signInToWorld(page, (p) => withUploads(p, reserved));
  await openGeneral(page);
  // The box writes the text in itself, so the paste is left to it.
  expect(
    await paste(page, {
      files: [{ name: "image.png", type: "image/png", png: true }],
      text: "Q3 figures",
    }),
  ).toBe(false);
  await expect(page.getByRole("list", { name: "Files to send" }).locator("img")).toBeVisible();
});

test("a copied file keeps its name, and the text naming it is left out", async ({ page }) => {
  const reserved: Reserved[] = [];
  await signInToWorld(page, (p) => withUploads(p, reserved));
  await openGeneral(page);
  const box = page.getByRole("textbox", { name: "Message" });
  expect(
    await paste(page, {
      files: [{ name: "report.pdf", type: "application/pdf" }],
      text: "file:///home/someone/report.pdf",
    }),
  ).toBe(true);
  await expect(
    page.getByRole("list", { name: "Files to send" }).getByText("report.pdf"),
  ).toBeVisible();
  await expect(box).toHaveValue("");
});

test("files and text dropped on the channel are attached and written in", async ({ page }) => {
  const reserved: Reserved[] = [];
  await signInToWorld(page, (p) => withUploads(p, reserved));
  await openGeneral(page);
  await drop(page, {
    files: [
      { name: "notes.txt", type: "text/plain" },
      { name: "image.png", type: "image/png", png: true },
    ],
  });
  const files = page.getByRole("list", { name: "Files to send" });
  await expect(files.getByText("notes.txt")).toBeVisible();
  await expect(files.getByText(TIMESTAMPED_PNG)).toBeVisible();

  await drop(page, { files: [], text: "https://example.org/page" });
  await expect(page.getByRole("textbox", { name: "Message" })).toHaveValue(
    "https://example.org/page",
  );
});

test("a picture still uploading can be seen whole, and beside its description", async ({
  page,
}) => {
  await signInToWorld(page, (p) => withUploads(p, [], true));
  await openGeneral(page);
  await paste(page, { files: [{ name: "cat.png", type: "image/png", png: true }] });
  const files = page.getByRole("list", { name: "Files to send" });
  await expect(files.getByRole("progressbar")).toBeVisible();

  await files.getByRole("button", { name: "View cat.png" }).click();
  const gallery = page.getByRole("dialog", { name: "Images" });
  await expect(gallery.getByRole("img").first()).toHaveAttribute("src", /^blob:/);
  await page.keyboard.press("Escape");
  await expect(gallery).toBeHidden();

  await files.getByRole("button", { name: "Describe cat.png" }).click();
  const describing = page.getByRole("dialog");
  await expect(describing.locator("img")).toHaveAttribute("src", /^blob:/);
  await expect(describing.getByRole("textbox", { name: "Description" })).toBeFocused();
});
