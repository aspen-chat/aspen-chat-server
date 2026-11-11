// @vitest-environment jsdom
import { ServerClock, type Poll } from "@aspen/protocol";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { pollOpen } from "./poll";
import { usePollNow } from "./usePollNow";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** A poll whose server, its clock at `serverNow`, closes it `closesInMs` from then. */
function poll(serverNow: number, closesInMs: number): Poll {
  return {
    id: "p",
    channelId: "c",
    messageId: "m",
    createdBy: "u",
    createdAt: new Date(serverNow - 60_000).toISOString(),
    closesAt: new Date(serverNow + closesInMs).toISOString(),
    closedAt: null,
    question: "q",
    options: [{ label: "a" }, { label: "b" }],
    multipleChoice: false,
    allowWriteIns: false,
    writeIns: [],
    anonymous: true,
    results: [{ count: 0 }, { count: 0 }],
  };
}

/** The clock of a server at `serverNow` as this device, at `Date.now()`, hears it. */
function heard(serverNow: number): ServerClock {
  const clock = new ServerClock();
  clock.note(new Date(serverNow).toISOString());
  return clock;
}

/** Whether a card would offer voting on `shown`, by the time the hook gives it. */
function offersVoting(shown: Poll, clock: ServerClock): boolean {
  let open = false;
  function Card() {
    open = pollOpen(shown, usePollNow(shown, clock));
    return null;
  }
  const root = createRoot(document.createElement("div"));
  act(() => {
    root.render(<Card />);
  });
  act(() => {
    root.unmount();
  });
  return open;
}

describe("usePollNow", () => {
  it("keeps a poll open until its server closes it, whatever the device's clock says", () => {
    // The device's clock is two hours ahead of the server's; the poll has an hour left.
    const behind = Date.now() - 7_200_000;
    expect(offersVoting(poll(behind, 3_600_000), heard(behind))).toBe(true);
    // The device's clock is two hours behind; the poll closed a minute ago.
    const ahead = Date.now() + 7_200_000;
    expect(offersVoting(poll(ahead, -60_000), heard(ahead))).toBe(false);
  });
});
