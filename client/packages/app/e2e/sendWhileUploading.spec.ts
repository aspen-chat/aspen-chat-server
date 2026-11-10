import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { general, me, settleAnimations, signInToWorld } from "./world";

/**
 * Sending a message while its file is still uploading, against the stubbed world: the message
 * goes to the server at once, naming the attachment reserved for the file, and stands at the
 * foot of the list, faded, until the upload ends and the server says it was posted. Meanwhile
 * it can be taken back into the box or deleted, and an upload that fails is said on it and sent
 * again to another link.
 */

const attachmentId = "0190f0a0-0000-7000-8004-000000000001";
const heldId = "0190f0a0-0000-7000-8004-0000000000a1";
const messageId = "0190f0a0-0000-7000-8004-0000000000b1";

interface World {
  /** Each request the attachment and message routes answered, in order. */
  calls: string[];
  /** What the message was sent as. */
  sent: { content: string; attachments: string[]; awaitUploads?: boolean }[];
  /** Whether the server refuses the message, as it does one its sender may not post. */
  refuse: boolean;
  /** How many uploads storage holds, waiting to be let through. */
  held: () => number;
  /** Lets every upload held at storage through, or, with `false`, fails them. */
  release: (ok?: boolean) => void;
}

/**
 * Answers the upload and the message as a deployment would, holding the file at storage until
 * the test lets it through. What the server tells by event, the tests publish themselves.
 */
async function withSlowStorage(page: Page): Promise<World> {
  const waiting: ((ok: boolean) => void)[] = [];
  const world: World = {
    calls: [],
    sent: [],
    refuse: false,
    held: () => waiting.length,
    release: (ok = true) => {
      for (const go of waiting.splice(0)) {
        go(ok);
      }
    },
  };
  const record = {
    id: attachmentId,
    fileName: "notes.txt",
    mimeType: "text/plain",
    downloadUrl: "https://media.test/notes.txt",
  };
  const handle = (origin: string, again: boolean) => ({
    id: attachmentId,
    uploadUrl: `${origin}/api/v1/uploads/attachment${again ? "?again" : ""}`,
    expiresAt: new Date(Date.now() + 600_000).toISOString(),
    contentType: "text/plain",
  });
  await page.route(
    /\/api\/v1\/(attachments|uploads|users\/@me\/held-messages|channels\/[^/]+\/messages)/,
    async (route) => {
      const request = route.request();
      const url = new URL(request.url());
      const path = url.pathname.replace("/api/v1", "");
      const method = request.method();
      if (method === "POST" && path === "/attachments") {
        world.calls.push("reserve");
        return route.fulfill({ status: 201, json: handle(url.origin, false) });
      }
      if (method === "POST" && path === `/attachments/${attachmentId}/renew`) {
        world.calls.push("renew");
        return route.fulfill({ json: handle(url.origin, true) });
      }
      if (method === "PUT" && path === "/uploads/attachment") {
        const ok = await new Promise<boolean>((go) => waiting.push(go));
        return ok ? route.fulfill({ status: 200, body: "" }) : route.abort("connectionfailed");
      }
      if (method === "POST" && path === `/attachments/${attachmentId}/confirm`) {
        world.calls.push("confirm");
        return route.fulfill({ json: record });
      }
      if (method === "DELETE" && path === `/attachments/${attachmentId}`) {
        world.calls.push("delete file");
        return route.fulfill({ status: 204, body: "" });
      }
      if (method === "DELETE" && path === `/users/@me/held-messages/${heldId}`) {
        world.calls.push("cancel");
        return route.fulfill({ status: 204, body: "" });
      }
      if (method === "POST" && path === `/channels/${general}/messages`) {
        const body = request.postDataJSON() as World["sent"][number];
        world.calls.push("send");
        world.sent.push(body);
        if (world.refuse) {
          return route.fulfill({
            status: 403,
            contentType: "application/problem+json",
            json: {
              code: "forbidden",
              title: "Forbidden",
              status: 403,
              detail: "You can't send messages here.",
            },
          });
        }
        return route.fulfill({
          status: 202,
          json: {
            id: heldId,
            channelId: general,
            content: body.content,
            attachments: body.attachments,
            echoToParent: false,
            replyTo: null,
            tagReplied: false,
            heldAt: new Date().toISOString(),
            waiting: "uploads",
            kind: "message",
            target: null,
            awaitedUploads: true,
          },
        });
      }
      return route.fallback();
    },
  );
  return world;
}

async function openGeneral(page: Page) {
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await expect(page.getByRole("textbox", { name: "Message" })).toBeVisible();
}

/** Chooses a text file for the box, which begins uploading it. */
async function attach(page: Page) {
  await page.locator('input[type="file"]').setInputFiles({
    name: "notes.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("some notes"),
  });
  await expect(page.getByRole("progressbar", { name: "Uploading notes.txt" })).toBeVisible();
}

/** Lets the upload storage holds through, or fails it, once it has got there. */
async function settleUpload(world: World | undefined, ok = true) {
  await expect.poll(() => world?.held()).toBe(1);
  world?.release(ok);
}

/** Sends what the box holds as a person would: Enter with a keyboard, the button on a phone. */
async function send(page: Page, isMobile: boolean) {
  if (isMobile) {
    await page.getByRole("button", { name: "Send", exact: true }).click();
  } else {
    await page.getByRole("textbox", { name: "Message" }).focus();
    await page.keyboard.press("Enter");
  }
}

const unsent = (page: Page) => page.locator("[data-pending-message]");

test("a message sent while its file uploads waits in the list until it is posted", async ({
  page,
  isMobile,
}) => {
  let world: World | undefined;
  const publish = await signInToWorld(page, async (p) => {
    world = await withSlowStorage(p);
  });
  await openGeneral(page);
  await attach(page);
  const box = page.getByRole("textbox", { name: "Message" });
  await box.fill("the notes from **today**");
  await expect(page.getByRole("button", { name: "Send", exact: true })).toBeEnabled();
  await send(page, isMobile);

  const row = unsent(page);
  await expect(row).toHaveCount(1);
  await expect(box).toHaveValue("");
  expect(world?.sent).toEqual([
    {
      content: "the notes from **today**",
      attachments: [attachmentId],
      mayHold: true,
      awaitUploads: true,
    },
  ]);
  // Drawn as the message it will be, faded, with its file on its way and what may be done.
  await expect(row.locator("strong")).toHaveText("today");
  await expect(row.locator(".not-yet-sent")).toHaveCSS("opacity", "0.8");
  await expect(row.getByRole("status")).toHaveText("Sending once its files have uploaded…");
  await expect(row.getByRole("progressbar", { name: "Uploading notes.txt" })).toBeVisible();
  await expect(row.getByRole("button", { name: "Edit" })).toBeVisible();
  await expect(row.getByRole("button", { name: "Delete" })).toBeVisible();
  // Nothing of it waits above the box, where what the channel holds up waits.
  await expect(page.getByRole("list", { name: "Messages waiting to send" })).toHaveCount(0);
  await settleAnimations(page);
  // Faded on purpose, so its contrast is not the page's; everything else about it is checked.
  const audit = await new AxeBuilder({ page })
    .include("[data-pending-message]")
    .disableRules(["color-contrast"])
    .analyze();
  expect(audit.violations).toEqual([]);
  // A reader who asked for more contrast has it at full strength, and its contrast with it.
  await page.evaluate(() => {
    document.documentElement.dataset.contrast = "more";
  });
  await expect(row.locator(".not-yet-sent")).toHaveCSS("opacity", "1");
  const plain = await new AxeBuilder({ page }).include("[data-pending-message]").analyze();
  expect(plain.violations).toEqual([]);

  // The file arrives and is confirmed; the server then says what the message became.
  await settleUpload(world);
  await expect.poll(() => world?.calls.at(-1)).toBe("confirm");
  await expect(row.getByText("Uploaded", { exact: true })).toBeVisible();
  publish({ serverEvent: "heldMessagePosted", held: heldId, channel: general, message: messageId });
  publish({
    serverEvent: "message",
    type: "create",
    id: messageId,
    channelId: general,
    author: me,
    content: "the notes from **today**",
    timestamp: new Date().toISOString(),
    editedAt: null,
    attachments: [attachmentId],
    linkPreviews: [],
    linkedMessages: [],
    alteredBy: [],
    kind: "standard",
    poll: null,
    thread: null,
    echoOf: null,
    mentions: { users: [], roles: [], everyone: false },
  });
  await expect(row).toHaveCount(0);
  await expect(page.locator("article").filter({ hasText: "the notes from today" })).toHaveCount(1);
});

test("an unsent message is taken back into the box, its file still uploading there", async ({
  page,
  isMobile,
}) => {
  let world: World | undefined;
  await signInToWorld(page, async (p) => {
    world = await withSlowStorage(p);
  });
  await openGeneral(page);
  await attach(page);
  const box = page.getByRole("textbox", { name: "Message" });
  await box.fill("on second thought");
  await send(page, isMobile);
  await unsent(page).getByRole("button", { name: "Edit" }).click();

  await expect(unsent(page)).toHaveCount(0);
  await expect(box).toHaveValue("on second thought");
  expect(world?.calls).toContain("cancel");
  // The same upload goes on as the box's own, and is not begun again.
  const files = page.getByRole("list", { name: "Files to send" });
  await expect(files.getByRole("progressbar", { name: "Uploading notes.txt" })).toBeVisible();
  await settleUpload(world);
  await expect(files.getByRole("progressbar")).toHaveCount(0);
  expect(world?.calls.filter((call) => call === "reserve")).toHaveLength(1);
  expect(world?.calls).not.toContain("delete file");
});

test("a message the server refuses stays in the box, its file still uploading there", async ({
  page,
  isMobile,
}) => {
  let world: World | undefined;
  await signInToWorld(page, async (p) => {
    world = await withSlowStorage(p);
    world.refuse = true;
  });
  await openGeneral(page);
  await attach(page);
  const box = page.getByRole("textbox", { name: "Message" });
  await box.fill("not allowed");
  await send(page, isMobile);

  await expect(
    page.getByRole("alert").filter({ hasText: "You can't send messages here." }),
  ).toBeVisible();
  await expect(unsent(page)).toHaveCount(0);
  await expect(box).toHaveValue("not allowed");
  const files = page.getByRole("list", { name: "Files to send" });
  await expect(files.getByRole("progressbar", { name: "Uploading notes.txt" })).toBeVisible();
  // The upload is the box's again: it ends there, and is not deleted.
  await settleUpload(world);
  await expect(files.getByRole("progressbar")).toHaveCount(0);
  await expect(files.getByText("notes.txt")).toBeVisible();
  expect(world?.calls).not.toContain("delete file");
});

test("an unsent message that is deleted takes its upload with it", async ({ page, isMobile }) => {
  let world: World | undefined;
  await signInToWorld(page, async (p) => {
    world = await withSlowStorage(p);
  });
  await openGeneral(page);
  await attach(page);
  await page.getByRole("textbox", { name: "Message" }).fill("never mind");
  await send(page, isMobile);
  await unsent(page).getByRole("button", { name: "Delete" }).click();

  await expect(unsent(page)).toHaveCount(0);
  await expect.poll(() => world?.calls).toContain("delete file");
  expect(world?.calls).toContain("cancel");
});

test("an upload that fails is said on the message, and sent again to another link", async ({
  page,
  isMobile,
}) => {
  let world: World | undefined;
  await signInToWorld(page, async (p) => {
    world = await withSlowStorage(p);
  });
  await openGeneral(page);
  await attach(page);
  await page.getByRole("textbox", { name: "Message" }).fill("a rocky road");
  await send(page, isMobile);
  const row = unsent(page);
  await expect(row).toHaveCount(1);

  await settleUpload(world, false);
  await expect(row.getByRole("alert")).toHaveText(
    "Not sent: notes.txt could not be uploaded. Check your connection, then try again.",
  );
  await expect(row.getByRole("status")).toHaveCount(0);
  // Said in passing too, for a reader who has gone elsewhere since sending it.
  await expect(
    page.getByText("A message was not sent: notes.txt could not be uploaded. Open its channel"),
  ).toBeVisible();
  await row.getByRole("button", { name: "Try again" }).click();
  await expect(row.getByRole("alert")).toHaveCount(0);
  await expect.poll(() => world?.calls.at(-1)).toBe("renew");
  await settleUpload(world);
  await expect.poll(() => world?.calls.at(-1)).toBe("confirm");
  // The same attachment throughout, so the message the server holds needs no change.
  expect(world?.calls.filter((call) => call === "reserve" || call === "send")).toEqual([
    "reserve",
    "send",
  ]);
});

test("a message another device is still uploading is shown, and may only be deleted", async ({
  page,
}) => {
  const publish = await signInToWorld(page, async (p) => {
    await withSlowStorage(p);
  });
  await openGeneral(page);
  publish({
    serverEvent: "heldMessageChanged",
    held: {
      id: heldId,
      channelId: general,
      content: "sent from my phone",
      attachments: [attachmentId],
      echoToParent: false,
      replyTo: null,
      tagReplied: false,
      heldAt: new Date().toISOString(),
      waiting: "uploads",
      kind: "message",
      target: null,
      awaitedUploads: true,
    },
  });
  const row = unsent(page);
  await expect(row).toContainText("sent from my phone");
  await expect(row).toContainText("Uploading from another device");
  await expect(row.getByRole("button", { name: "Edit" })).toHaveCount(0);
  await expect(row.getByRole("button", { name: "Delete" })).toBeVisible();
  // Cancelled there, it goes from here too.
  publish({ serverEvent: "heldMessageCancelled", held: heldId, channel: general });
  await expect(row).toHaveCount(0);
});
