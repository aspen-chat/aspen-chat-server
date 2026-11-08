import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { general, settleAnimations, signInToWorld, thread } from "./world";

/**
 * Scheduling a message for later, against the stubbed world: the message box's schedule control
 * and its dialog, the bar over the box that holds the time chosen until the message is sent, the
 * row above the box that counts what is scheduled there (and, apart and in the danger colour,
 * what could not be sent), and the list it opens, from which one is sent now, given another
 * time, taken back into the box, or deleted.
 */

interface Scheduled {
  id: string;
  channelId: string;
  content: string;
  attachments: string[];
  replyTo: string | null;
  tagReplied: boolean;
  sendAt: string;
  status: "scheduled" | "waitingForMana" | "failed";
  failure: string | null;
}

interface Asked {
  method: string;
  path: string;
  body: { content?: string; sendAt?: string } | null;
}

const scheduledId = (n: number) =>
  `0190f0a0-0000-7000-8003-0000000000${String(n).padStart(2, "0")}`;

/**
 * Answers the scheduled-message endpoints from `held`, as the server would, recording each
 * request in `asked`. What the server tells by event, the tests publish themselves.
 */
async function withScheduled(page: Page, held: Scheduled[], asked: Asked[]) {
  await page.route(/\/api\/v1\/(users\/@me|channels\/[^/]+)\/scheduled-messages/, (route) => {
    const request = route.request();
    const path = new URL(request.url()).pathname.replace("/api/v1", "");
    const method = request.method();
    const body = (request.postDataJSON() ?? null) as Asked["body"];
    asked.push({ method, path, body });
    if (method === "GET" && path === "/users/@me/scheduled-messages") {
      return route.fulfill({ json: held });
    }
    const posted = /^\/channels\/([^/]+)\/scheduled-messages$/.exec(path);
    if (method === "POST" && posted?.[1] !== undefined) {
      const made: Scheduled = {
        id: scheduledId(held.length + 1),
        channelId: posted[1],
        content: body?.content ?? "",
        attachments: [],
        replyTo: null,
        tagReplied: false,
        sendAt: body?.sendAt ?? "",
        status: "scheduled",
        failure: null,
      };
      held.push(made);
      return route.fulfill({ status: 201, json: made });
    }
    const [, id, action] = /^\/users\/@me\/scheduled-messages\/([^/]+)(\/send)?$/.exec(path) ?? [];
    const at = held.findIndex((message) => message.id === id);
    const found = held[at];
    if (found === undefined) {
      return route.fulfill({
        status: 404,
        contentType: "application/problem+json",
        json: { code: "notFound", title: "Not found", status: 404 },
      });
    }
    if (method === "GET") {
      return route.fulfill({ json: { data: found, included: { attachments: [] } } });
    }
    if (method === "DELETE") {
      held.splice(at, 1);
      return route.fulfill({ status: 204, body: "" });
    }
    if (method === "PATCH" || action === "/send") {
      const next: Scheduled = {
        ...found,
        sendAt: method === "PATCH" ? (body?.sendAt ?? "") : new Date().toISOString(),
        status: "scheduled",
        failure: null,
      };
      held[at] = next;
      return route.fulfill({ json: next });
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
  await expect(page.getByRole("textbox", { name: "Message" })).toBeVisible();
}

/**
 * The schedule control of the box in `scope` (the page's main box unless given): its own button
 * on a wide screen, an item of the + menu on a narrow one.
 */
async function scheduleControl(page: Page, scope: Page | Locator = page) {
  const button = scope.getByRole("button", { name: "Schedule message" });
  if (await button.isVisible()) {
    return button;
  }
  await scope
    .getByRole("button", { name: "Add a file or a poll, or schedule this message" })
    .click();
  return page.getByRole("menuitem", { name: "Schedule message" });
}

/** Opens the channel named, from the channel list, which a phone goes back to first. */
async function openChannel(page: Page, name: string) {
  const back = page.getByRole("link", { name: "Back to channels" });
  if (await back.isVisible()) {
    await back.click();
  }
  await page.getByText(name, { exact: true }).click();
}

const scheduled = (asked: Asked[]) => asked.filter((request) => request.method === "POST");

const strip = (page: Page) => page.locator("[data-scheduled-strip]");
const bar = (page: Page) => page.locator("[data-schedule-bar]");

/** Opens the schedule dialog of the box in `scope`, settled, so its choices take a press. */
async function openSchedule(page: Page, scope: Page | Locator = page) {
  await (await scheduleControl(page, scope)).click();
  const dialog = page.getByRole("dialog", { name: "Schedule message" });
  await expect(dialog).toBeVisible();
  await settleAnimations(page);
  return dialog;
}

/** Opens the schedule dialog of the box in `scope` and takes the time it offers first, an hour from now. */
async function setTime(page: Page, scope: Page | Locator = page) {
  const dialog = await openSchedule(page, scope);
  await dialog.getByRole("button", { name: "Set time", exact: true }).click();
  await expect(dialog).toBeHidden();
}

/**
 * Sends what the box in `scope` holds as a person would: Enter with a keyboard, the Schedule
 * button on a phone, where Enter breaks a line.
 */
async function sendScheduled(page: Page, isMobile: boolean, scope: Page | Locator = page) {
  if (isMobile) {
    await scope.getByRole("button", { name: "Schedule", exact: true }).click();
  } else {
    await scope.getByRole("textbox", { name: "Message" }).focus();
    await page.keyboard.press("Enter");
  }
}

function failedOne(n: number): Scheduled {
  return {
    id: scheduledId(n),
    channelId: general,
    content: "the plan has changed",
    attachments: [],
    replyTo: null,
    tagReplied: false,
    sendAt: new Date(Date.now() - 3_600_000).toISOString(),
    status: "failed",
    failure: "You can no longer post here.",
  };
}

async function audit(page: Page): Promise<string[]> {
  await settleAnimations(page);
  // A popover or modal is rendered outside every landmark, as a floating layer must be.
  const found = await new AxeBuilder({ page }).disableRules(["region"]).analyze();
  return found.violations.flatMap((violation) =>
    violation.nodes.map((node) => `${violation.id} at ${node.html.slice(0, 200)}`),
  );
}

test("a message is scheduled from the box and counted above it", async ({ page, isMobile }) => {
  const held: Scheduled[] = [];
  const asked: Asked[] = [];
  await signInToWorld(page, (p) => withScheduled(p, held, asked));
  const sent: string[] = [];
  page.on("request", (request) => {
    if (request.method() === "POST" && request.url().endsWith(`/channels/${general}/messages`)) {
      sent.push(request.url());
    }
  });
  await openGeneral(page);
  await expect(strip(page)).toHaveCount(0);
  const box = page.getByRole("textbox", { name: "Message" });
  const send = page.getByRole("button", { name: "Send", exact: true });
  const scheduleSend = page.getByRole("button", { name: "Schedule", exact: true });

  // A time is chosen before anything is written, and shown over the box until the message is
  // sent; Send becomes Schedule meanwhile. The bar's X sends now after all.
  const dialog = await openSchedule(page);
  await expect(dialog.getByRole("radio", { name: /In an hour/ })).toBeChecked();
  expect(await audit(page)).toEqual([]);
  await dialog.getByRole("radio", { name: /Tomorrow morning/ }).check({ force: true });
  await dialog.getByRole("button", { name: "Set time", exact: true }).click();
  await expect(dialog).toBeHidden();
  await expect(bar(page)).toContainText("Scheduling a message for");
  await expect(scheduleSend).toBeVisible();
  await expect(send).toHaveCount(0);
  expect(scheduled(asked)).toEqual([]);
  expect(await audit(page)).toEqual([]);
  await bar(page).getByRole("button", { name: "Don't schedule" }).click();
  await expect(bar(page)).toHaveCount(0);
  await expect(send).toBeVisible();
  await expect(box).toBeFocused();

  // Chosen again with the message written, the dialog opens on the time already held; Enter
  // then schedules rather than sends.
  await box.fill("see you tomorrow");
  await openSchedule(page);
  await dialog.getByRole("radio", { name: /Tomorrow morning/ }).check({ force: true });
  await dialog.getByRole("button", { name: "Set time", exact: true }).click();
  await expect(dialog).toBeHidden();
  await openSchedule(page);
  await expect(dialog.getByRole("radio", { name: "Another time" })).toBeChecked();
  const expectedTime = await page.evaluate(() => {
    const tomorrow = new Date();
    tomorrow.setDate(tomorrow.getDate() + 1);
    tomorrow.setHours(9, 0, 0, 0);
    return { iso: tomorrow.toISOString(), day: tomorrow.getDate() };
  });
  await expect(
    dialog.getByRole("group", { name: "Date and time" }).getByRole("spinbutton", { name: /^day/ }),
  ).toHaveText(String(expectedTime.day));
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(bar(page)).toBeVisible();
  await sendScheduled(page, isMobile);

  // Sent as written, for nine tomorrow morning on the reader's own clock, and the box is empty.
  const posted = asked.find((request) => request.method === "POST");
  expect(posted?.body?.content).toBe("see you tomorrow");
  expect(posted?.body?.sendAt).toBe(expectedTime.iso);
  await expect(box).toHaveValue("");
  await expect(bar(page)).toHaveCount(0);
  await expect(send).toBeVisible();
  await expect(strip(page)).toContainText("1 message scheduled here");
  await expect(page.locator("[data-scheduled-failures]")).toHaveCount(0);
  // Scheduled, not also sent.
  expect(sent).toEqual([]);
  await expect(page.getByText("see you tomorrow")).toHaveCount(0);
});

test("a time of the reader's own is checked before it is asked for", async ({ page, isMobile }) => {
  const held: Scheduled[] = [];
  const asked: Asked[] = [];
  await signInToWorld(page, (p) => withScheduled(p, held, asked));
  await openGeneral(page);
  const box = page.getByRole("textbox", { name: "Message" });
  await box.fill("much later");
  const dialog = await openSchedule(page);
  await dialog.getByRole("radio", { name: "Another time" }).check({ force: true });
  const field = dialog.getByRole("group", { name: "Date and time" });
  const submit = dialog.getByRole("button", { name: "Set time", exact: true });
  await expect(submit).toBeDisabled();

  // Typed segment by segment, as the app's English writes a date: month, day, year, then the
  // hour, minute, and half of the day.
  async function type(at: Date) {
    const digits: Record<string, string> = {
      month: String(at.getMonth() + 1),
      day: String(at.getDate()),
      year: String(at.getFullYear()),
      hour: String(at.getHours() % 12 || 12),
      minute: String(at.getMinutes()).padStart(2, "0"),
    };
    for (const [name, value] of Object.entries(digits)) {
      const segment = field.getByRole("spinbutton", { name: new RegExp(`^${name}\\b`) });
      await segment.click();
      await segment.pressSequentially(value);
    }
    const half = field.getByRole("spinbutton", { name: /^AM\/PM/ });
    await half.click();
    await half.pressSequentially(at.getHours() < 12 ? "A" : "P");
  }
  await type(new Date(2020, 0, 1, 9, 0));
  await submit.click();
  await expect(dialog.getByRole("alert")).toContainText("at least 1 minute from now");
  expect(asked.some((request) => request.method === "POST")).toBe(false);

  const chosen = await page.evaluate(() => {
    const at = new Date(Date.now() + 3 * 86_400_000);
    at.setHours(14, 30, 0, 0);
    return { time: at.getTime(), iso: at.toISOString() };
  });
  await type(new Date(chosen.time));
  await submit.click();
  await expect(dialog).toBeHidden();
  await expect(bar(page)).toBeVisible();
  await sendScheduled(page, isMobile);
  await expect(bar(page)).toHaveCount(0);
  expect(scheduled(asked)[0]?.body?.sendAt).toBe(chosen.iso);
});

test("what could not be sent is counted apart, in the danger colour", async ({ page }) => {
  const held: Scheduled[] = [failedOne(1)];
  const asked: Asked[] = [];
  await signInToWorld(page, (p) => withScheduled(p, held, asked));
  await openGeneral(page);

  // One failed and none waiting: only the failure is counted.
  const failures = page.locator("[data-scheduled-failures]");
  await expect(failures).toHaveText("1 could not be sent");
  await expect(strip(page)).not.toContainText("scheduled here");
  const danger = await page.evaluate(() => {
    const probe = document.createElement("span");
    probe.className = "text-danger";
    document.body.append(probe);
    const colour = getComputedStyle(probe).color;
    probe.remove();
    return colour;
  });
  await expect(failures).toHaveCSS("color", danger);
  await expect(failures.locator("svg")).toBeVisible();

  // One more scheduled: both counts, each its own.
  await page.getByRole("textbox", { name: "Message" }).fill("and another");
  await setTime(page);
  await page.getByRole("button", { name: "Schedule", exact: true }).click();
  await expect(strip(page)).toContainText("1 message scheduled here");
  await expect(failures).toHaveText("1 could not be sent");
  expect(await audit(page)).toEqual([]);
});

test("the list sends one now, gives one another time, and deletes one", async ({ page }) => {
  const held: Scheduled[] = [
    failedOne(1),
    { ...failedOne(2), content: "good morning", status: "scheduled", failure: null },
    { ...failedOne(3), content: "the busy one", status: "scheduled", failure: null },
  ];
  const asked: Asked[] = [];
  const publish = await signInToWorld(page, (p) => withScheduled(p, held, asked));
  await openGeneral(page);
  await expect(strip(page)).toContainText("2 messages scheduled here");
  await strip(page).click();
  const dialog = page.getByRole("dialog", { name: "Scheduled messages" });
  const rows = dialog.getByRole("listitem");
  await expect(rows).toHaveCount(3);
  const failed = dialog.locator(`[data-scheduled-message="${scheduledId(1)}"]`);
  await expect(failed).toContainText("Could not be sent: You can no longer post here.");
  expect(await audit(page)).toEqual([]);

  // Its time comes in a busy channel: it says so, and is already being sent.
  const busy = dialog.locator(`[data-scheduled-message="${scheduledId(3)}"]`);
  publish({
    serverEvent: "scheduledMessageChanged",
    scheduled: { ...held[2], status: "waitingForMana" },
  });
  await expect(busy).toContainText("Waiting for mana");
  await expect(busy.getByRole("button", { name: "Send now" })).toHaveCount(0);
  await expect(busy.getByRole("button", { name: "Change time" })).toBeVisible();
  await expect(busy.getByRole("button", { name: "Delete" })).toBeVisible();

  // Another time for the failed one, in a second step of the same dialog.
  await failed.getByRole("button", { name: "Change time" }).click();
  const change = page.getByRole("dialog", { name: "Change time" });
  await change.getByRole("button", { name: "Change time", exact: true }).click();
  const patched = asked.find((request) => request.method === "PATCH");
  expect(patched?.path).toBe(`/users/@me/scheduled-messages/${scheduledId(1)}`);
  expect(typeof patched?.body?.sendAt).toBe("string");
  publish({ serverEvent: "scheduledMessageChanged", scheduled: held[0] });
  await expect(dialog.getByRole("heading", { name: "Scheduled messages" })).toBeVisible();
  await expect(failed).not.toContainText("Could not be sent");

  // Sent now, it leaves once the stream says it was posted.
  const morning = dialog.locator(`[data-scheduled-message="${scheduledId(2)}"]`);
  await morning.getByRole("button", { name: "Send now" }).click();
  await expect
    .poll(() => asked.some((request) => request.path.endsWith(`${scheduledId(2)}/send`)))
    .toBe(true);
  publish({
    serverEvent: "heldMessagePosted",
    held: scheduledId(2),
    channel: general,
    message: "0190f0a0-0000-7000-8003-000000000099",
  });
  await expect(rows).toHaveCount(2);

  // Deleted, each goes at once.
  await busy.getByRole("button", { name: "Delete" }).click();
  await expect(rows).toHaveCount(1);
  await failed.getByRole("button", { name: "Delete" }).click();
  await expect(dialog.getByText("Nothing is scheduled here.")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(strip(page)).toHaveCount(0);
});

test("editing takes a scheduled message back into the box, its time with it", async ({ page }) => {
  const tomorrow = new Date(Date.now() + 86_400_000);
  const held: Scheduled[] = [
    {
      ...failedOne(1),
      content: "a first draft",
      status: "scheduled",
      failure: null,
      sendAt: tomorrow.toISOString(),
    },
    { ...failedOne(2), content: "too late now" },
  ];
  const asked: Asked[] = [];
  await signInToWorld(page, (p) => withScheduled(p, held, asked));
  await openGeneral(page);
  const box = page.getByRole("textbox", { name: "Message" });

  // Not while the box holds something else, which it would replace.
  await box.fill("already writing");
  await strip(page).click();
  const dialog = page.getByRole("dialog", { name: "Scheduled messages" });
  const row = (n: number) => dialog.locator(`[data-scheduled-message="${scheduledId(n)}"]`);
  await row(1).getByRole("button", { name: "Edit" }).click();
  await expect(dialog.getByRole("alert")).toContainText("already holds a message");
  expect(asked.some((request) => request.method === "DELETE")).toBe(false);
  await page.keyboard.press("Escape");

  await box.fill("");
  await strip(page).click();
  await row(1).getByRole("button", { name: "Edit" }).click();
  await expect(dialog).toBeHidden();
  await expect(box).toHaveValue("a first draft");
  expect(
    asked
      .filter((request) => request.path.endsWith(scheduledId(1)))
      .map((request) => request.method),
  ).toEqual(["GET", "DELETE"]);
  // Its time is held over the box, so sending schedules it again.
  await expect(bar(page)).toContainText("Scheduling a message for");
  await expect(bar(page)).toContainText(String(tomorrow.getDate()));
  await expect(page.getByRole("button", { name: "Schedule", exact: true })).toBeVisible();
  await bar(page).getByRole("button", { name: "Don't schedule" }).click();
  await box.fill("");

  // A failed one's time has passed, so none is held: it is sent now or given a time.
  await strip(page).click();
  await row(2).getByRole("button", { name: "Edit" }).click();
  await expect(dialog).toBeHidden();
  await expect(box).toHaveValue("too late now");
  await expect(bar(page)).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Send", exact: true })).toBeVisible();
  await expect(strip(page)).toHaveCount(0);
});

test("a time waits with the draft, and a refusal leaves it", async ({ page, isMobile }) => {
  const held: Scheduled[] = [];
  const asked: Asked[] = [];
  await signInToWorld(page, (p) => withScheduled(p, held, asked));
  await openGeneral(page);
  const box = page.getByRole("textbox", { name: "Message" });
  await box.fill("back in a moment");
  await setTime(page);
  await expect(bar(page)).toBeVisible();

  // Kept with the draft: still there on coming back to the channel.
  await openChannel(page, "roadmap");
  await expect(box).toHaveAttribute("aria-placeholder", "Message #roadmap");
  await expect(bar(page)).toHaveCount(0);
  await openChannel(page, "general");
  await expect(box).toHaveValue("back in a moment");
  await expect(bar(page)).toBeVisible();
  await bar(page).getByRole("button", { name: "Don't schedule" }).click();
  await box.fill("");

  // In a thread, a reply also shown in the channel cannot be scheduled: Enter says so over the
  // box and the time stays, for the reader to turn that off and try again.
  await page
    .getByRole("link", { name: /^View thread/ })
    .first()
    .click();
  const panel = page.locator("section[aria-label='Thread']");
  const threadBox = panel.getByRole("textbox", { name: "Message" });
  await threadBox.fill("one more thing");
  await panel.getByRole("checkbox", { name: /^Also send to/ }).check();
  await setTime(page, panel);
  await sendScheduled(page, isMobile, panel);
  await expect(panel.getByRole("alert")).toContainText("can't also be shown in");
  await expect(panel.locator("[data-schedule-bar]")).toBeVisible();
  await expect(threadBox).toHaveValue("one more thing");
  expect(scheduled(asked)).toEqual([]);
  await panel.getByRole("checkbox", { name: /^Also send to/ }).uncheck();
  await sendScheduled(page, isMobile, panel);
  await expect(threadBox).toHaveValue("");
  const posted = scheduled(asked)[0];
  expect(posted?.path).toBe(`/channels/${thread}/scheduled-messages`);
  expect(posted?.body?.content).toBe("one more thing");
});
