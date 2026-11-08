import AxeBuilder from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";
import { general, settleAnimations, signInToWorld } from "./world";

/**
 * Scheduling a message for later, against the stubbed world: the message box's schedule control
 * and its dialog, the row above the box that counts what is scheduled there (and, apart and in
 * the danger colour, what could not be sent), and the list it opens, from which one is sent now,
 * given another time, taken back into the box, or deleted.
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
    if (method === "POST" && path === `/channels/${general}/scheduled-messages`) {
      const made: Scheduled = {
        id: scheduledId(held.length + 1),
        channelId: general,
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

/** The box's schedule control: its own button on a wide screen, an item of the + menu on a narrow one. */
async function scheduleControl(page: Page) {
  const button = page.getByRole("button", { name: "Schedule message" });
  if (await button.isVisible()) {
    return button;
  }
  await page
    .getByRole("button", { name: "Add a file or a poll, or schedule this message" })
    .click();
  return page.getByRole("menuitem", { name: "Schedule message" });
}

const strip = (page: Page) => page.locator("[data-scheduled-strip]");

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

test("a message is scheduled from the box and counted above it", async ({ page }) => {
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

  // With nothing written there is nothing to schedule.
  const empty = await scheduleControl(page);
  await expect(empty).toBeDisabled();
  await page.keyboard.press("Escape");

  const box = page.getByRole("textbox", { name: "Message" });
  await box.fill("see you tomorrow");
  await (await scheduleControl(page)).click();
  const dialog = page.getByRole("dialog", { name: "Schedule message" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("radio", { name: /In an hour/ })).toBeChecked();
  expect(await audit(page)).toEqual([]);
  await dialog.getByRole("radio", { name: /Tomorrow morning/ }).check({ force: true });
  await dialog.getByRole("button", { name: "Schedule", exact: true }).click();
  await expect(dialog).toBeHidden();

  // Sent as written, for nine tomorrow morning on the reader's own clock, and the box is empty.
  const posted = asked.find((request) => request.method === "POST");
  expect(posted?.body?.content).toBe("see you tomorrow");
  const expected = await page.evaluate(() => {
    const tomorrow = new Date();
    tomorrow.setDate(tomorrow.getDate() + 1);
    tomorrow.setHours(9, 0, 0, 0);
    return tomorrow.toISOString();
  });
  expect(posted?.body?.sendAt).toBe(expected);
  await expect(box).toHaveValue("");
  await expect(strip(page)).toContainText("1 message scheduled here");
  await expect(page.locator("[data-scheduled-failures]")).toHaveCount(0);
  // Scheduled, not also sent: the dialog's form is not the box's.
  expect(sent).toEqual([]);
  await expect(page.getByText("see you tomorrow")).toHaveCount(0);
});

test("a time of the reader's own is checked before it is asked for", async ({ page }) => {
  const held: Scheduled[] = [];
  const asked: Asked[] = [];
  await signInToWorld(page, (p) => withScheduled(p, held, asked));
  await openGeneral(page);
  await page.getByRole("textbox", { name: "Message" }).fill("much later");
  await (await scheduleControl(page)).click();
  const dialog = page.getByRole("dialog", { name: "Schedule message" });
  await dialog.getByRole("radio", { name: "Another time" }).check({ force: true });
  const field = dialog.getByRole("group", { name: "Date and time" });
  const submit = dialog.getByRole("button", { name: "Schedule", exact: true });
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
  expect(asked.find((request) => request.method === "POST")?.body?.sendAt).toBe(chosen.iso);
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
  await (await scheduleControl(page)).click();
  await page
    .getByRole("dialog", { name: "Schedule message" })
    .getByRole("button", { name: "Schedule", exact: true })
    .click();
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

test("editing takes a scheduled message back into the box", async ({ page }) => {
  const held: Scheduled[] = [
    { ...failedOne(1), content: "a first draft", status: "scheduled", failure: null },
  ];
  const asked: Asked[] = [];
  await signInToWorld(page, (p) => withScheduled(p, held, asked));
  await openGeneral(page);
  const box = page.getByRole("textbox", { name: "Message" });

  // Not while the box holds something else, which it would replace.
  await box.fill("already writing");
  await strip(page).click();
  const dialog = page.getByRole("dialog", { name: "Scheduled messages" });
  await dialog.getByRole("button", { name: "Edit" }).click();
  await expect(dialog.getByRole("alert")).toContainText("already holds a message");
  expect(asked.some((request) => request.method === "DELETE")).toBe(false);
  await page.keyboard.press("Escape");

  await box.fill("");
  await strip(page).click();
  await dialog.getByRole("button", { name: "Edit" }).click();
  await expect(dialog).toBeHidden();
  await expect(box).toHaveValue("a first draft");
  expect(
    asked
      .filter((request) => request.path.endsWith(scheduledId(1)))
      .map((request) => request.method),
  ).toEqual(["GET", "DELETE"]);
  await expect(strip(page)).toHaveCount(0);
});
