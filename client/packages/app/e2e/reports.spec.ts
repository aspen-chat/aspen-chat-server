import { expect, test, type Page } from "@playwright/test";
import { longPress, reactedText, signInToWorld } from "./world";

/**
 * Reporting Bob's "Sounds good" in the stubbed world: the dialog offers the server's
 * categories with what each covers, Other asks for an explanation before it can be sent, and
 * a sent report says so in a toast. The report's request is checked here.
 */

const categories = [
  {
    id: "0190f0a0-0000-7000-8000-00000000c001",
    builtin: "spam",
    name: "Spam",
    description: "Unwanted advertising",
    hidden: false,
  },
  {
    id: "0190f0a0-0000-7000-8000-00000000c002",
    builtin: "other",
    name: "Other",
    description: "Something else",
    hidden: false,
  },
];

/** Opens "Sounds good"'s actions, hovered where there is a pointer and pressed long on a phone, and picks `action`. */
async function messageAction(page: Page, isMobile: boolean, action: string) {
  const message = page.locator("article").filter({ hasText: reactedText }).last();
  if (isMobile) {
    await longPress(page, message.locator(".message-body"));
    await page
      .getByRole("dialog", { name: "Message actions" })
      .getByRole("button", { name: action })
      .tap();
  } else {
    await message.hover();
    await message.getByRole("button", { name: action }).click();
  }
}

async function withReports(page: Page, sent: unknown[]) {
  await page.route(/\/api\/v1\/report-categories$/, (route) => route.fulfill({ json: categories }));
  await page.route(/\/api\/v1\/messages\/[^/]+\/reports$/, (route) => {
    sent.push(route.request().postDataJSON());
    return route.fulfill({
      status: 201,
      json: { id: "0190f0a0-0000-7000-8000-00000000c0ff", createdAt: new Date().toISOString() },
    });
  });
}

test("a message is reported in a category, and Other needs an explanation", async ({
  page,
  isMobile,
}) => {
  const sent: unknown[] = [];
  await signInToWorld(page, (p) => withReports(p, sent));
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await messageAction(page, isMobile, "Report message");
  const dialog = page.getByRole("dialog", { name: "Report this message" });
  await expect(dialog).toBeVisible();
  const send = dialog.getByRole("button", { name: "Send report" });
  await expect(send).toBeDisabled();
  await dialog.getByRole("button", { name: /Why are you reporting it/ }).click();
  await page.getByRole("option", { name: /Other/ }).click();
  await expect(send).toBeDisabled();
  await dialog.getByRole("textbox", { name: "What's wrong?" }).fill("Not a nice thing to say");
  await send.click();
  await expect(dialog).toBeHidden();
  await expect(page.getByText("Report sent. Thank you.")).toBeVisible();
  expect(sent).toEqual([{ category: categories[1]?.id, explanation: "Not a nice thing to say" }]);
});

test("a message's link is copied from its actions", async ({
  page,
  context,
  isMobile,
  browserName,
}) => {
  // Reading the clipboard back takes a permission only Chromium's driver grants.
  if (browserName === "chromium") {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  }
  await signInToWorld(page);
  await page
    .getByRole("grid", { name: "Channels" })
    .first()
    .getByText("general", { exact: true })
    .click();
  await messageAction(page, isMobile, "Copy link");
  await expect(page.getByText("Copied link to the message")).toBeVisible();
  if (browserName === "chromium") {
    const copied = await page.evaluate(() => navigator.clipboard.readText());
    expect(copied).toMatch(/\/communities\/[^/]+\/channels\/[^/]+\/messages\/[^/]+$/);
  }
});
