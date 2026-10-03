import { expect, test, type Page, type Request } from "@playwright/test";
import { bob, general, helper, me, signInToWorld, type Publish, submit } from "./world";

/**
 * Bots' commands in the message box, against the stubbed world in `world.ts`, where Kate's bot
 * Helper is a member of the community and answers the commands below in #general.
 */

const commands = [
  {
    bot: helper,
    commands: [
      {
        name: "roll",
        description: "Rolls dice",
        parameters: [
          { name: "dice", description: "Like 2d6", type: "regex", pattern: "[0-9]+d[0-9]+" },
          { name: "why", description: "What for", type: "any", optional: true },
        ],
      },
      {
        name: "poke",
        description: "Pokes someone",
        parameters: [{ name: "who", description: "Whom to poke", type: "userId" }],
      },
    ],
  },
];

async function withCommands(page: Page) {
  await page.route(`**/api/v1/channels/${general}/commands`, async (route) => {
    if (route.request().method() === "GET") {
      await route.fulfill({ json: commands });
    } else {
      await route.fulfill({ status: 201, json: {} });
    }
  });
}

const invoked = (page: Page) =>
  page.waitForRequest(
    (r: Request) => r.method() === "POST" && r.url().endsWith(`/channels/${general}/commands`),
  );

let publish: Publish;

test.beforeEach(async ({ page }) => {
  publish = await signInToWorld(page, withCommands);
  await page.getByText("general", { exact: true }).click();
});

test("typing / offers the bots' commands, and a person picked is sent as their id", async ({
  page,
}) => {
  const box = page.getByRole("textbox", { name: "Message" });
  await box.click();
  await page.keyboard.type("/");
  const list = page.getByRole("listbox", { name: "Bots' commands" });
  await expect(list.getByRole("option", { name: /^\/roll, Helper, Rolls dice/ })).toBeVisible();
  await expect(page.getByRole("status")).toHaveText(/^2 commands\./);
  await page.keyboard.type("po");
  await page.keyboard.press("Enter");
  await expect(box).toHaveValue("/poke ");

  // The form shows above the box, with what the parameter at the caret takes.
  await expect(page.getByText("Whom to poke")).toBeVisible();
  await page.keyboard.type("@bo");
  const people = page.getByRole("listbox", { name: "Suggestions for who" });
  await expect(people.getByRole("option", { name: /Bob With A Rather Long/ })).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(box).toHaveValue("/poke @bob ");
  const sent = invoked(page);
  await submit(page);
  expect((await sent).postDataJSON()).toEqual({
    bot: helper,
    name: "poke",
    arguments: [bob],
    attachments: [],
  });
  await expect(box).toHaveValue("");
});

test("a value its pattern refuses is flagged, and the last parameter takes the rest", async ({
  page,
}) => {
  const box = page.getByRole("textbox", { name: "Message" });
  await box.click();
  await page.keyboard.type("/roll x");
  await expect(page.getByText("dice doesn't take this.")).toBeVisible();
  await page.keyboard.press("Backspace");
  await page.keyboard.type("2d6 for  luck");
  await expect(page.getByText("dice doesn't take this.")).toBeHidden();
  const sent = invoked(page);
  await submit(page);
  expect((await sent).postDataJSON()).toMatchObject({ arguments: ["2d6", "for  luck"] });
});

test("a line no bot here answers is sent as a message", async ({ page }) => {
  const box = page.getByRole("textbox", { name: "Message" });
  await box.fill("/shrug hi");
  const sent = page.waitForRequest(
    (r) => r.method() === "POST" && r.url().endsWith(`/channels/${general}/messages`),
  );
  await submit(page);
  expect(((await sent).postDataJSON() as { content: string }).content).toBe("/shrug hi");
});

test("a command in the channel says whose it was", async ({ page }) => {
  publish({
    serverEvent: "message",
    type: "create",
    id: "0190f0a0-0000-7000-8001-000000000999",
    channelId: general,
    author: me,
    content: `/poke <@${bob}>`,
    timestamp: new Date().toISOString(),
    editedAt: null,
    linkPreviews: [],
    linkedMessages: [],
    kind: "command",
    commandBot: helper,
    poll: null,
    thread: null,
    echoOf: null,
    attachments: [],
    mentions: { users: [], roles: [], everyone: false },
  });
  const record = page.locator("article").filter({ hasText: "/poke" });
  await expect(record).toContainText("sent @Helper a command");
  // The person it names reads by name, and is tagged by no one.
  await expect(record).toContainText("/poke @Bob With A Rather Long Display Name");
  await expect(record).not.toHaveAttribute("data-mentions-me");
});
