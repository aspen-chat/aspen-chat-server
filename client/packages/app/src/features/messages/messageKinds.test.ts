import type { MessageKind } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { kindActions } from "./messageKinds";

describe("kindActions", () => {
  it("lets what people write start threads and be reported", () => {
    for (const kind of ["standard", "poll", "command"] as const) {
      expect(kindActions(kind)).toEqual({ threadable: true, reportable: true });
    }
  });

  it("starts no thread on a record of something that happened, or on an echo", () => {
    for (const kind of [
      "pollClosed",
      "threadEcho",
      "call",
      "missedCall",
      "memberJoined",
      "communityGrew",
    ] as const) {
      expect(kindActions(kind).threadable).toBe(false);
    }
  });

  it("offers no report of a call's record or a notice of who joined", () => {
    for (const kind of ["call", "missedCall", "memberJoined", "communityGrew"] as const) {
      expect(kindActions(kind).reportable).toBe(false);
    }
    expect(kindActions("threadEcho").reportable).toBe(true);
  });

  it("allows nothing of a kind this build does not know", () => {
    expect(kindActions("announcement" as MessageKind)).toEqual({
      threadable: false,
      reportable: false,
    });
  });
});
