import type { Channel, User } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { dmTitle, otherRecipients } from "./dmName";

const user = (id: string, name: string, displayName?: string): User => ({
  id,
  name,
  icon: null,
  onlineStatus: "offline",
  ...(displayName === undefined ? {} : { displayName }),
});

describe("dmTitle", () => {
  const channel: Channel = {
    id: "dm",
    community: null,
    parentCategory: null,
    name: "",
    sortIndex: 0,
    ty: "groupDm",
    replyCount: 0,
    recipients: ["me", "b", "c"],
  };

  it("names a DM after everyone but the caller, in the order they joined", () => {
    expect(otherRecipients(channel, "me")).toEqual(["b", "c"]);
    expect(dmTitle([user("b", "bob", "Bob"), user("c", "carol")], "Unknown", "Nobody")).toBe(
      "Bob, carol",
    );
  });

  it("stands in for people not yet loaded, and for a DM with nobody else left", () => {
    expect(dmTitle([undefined, user("c", "carol")], "Unknown", "Nobody")).toBe("Unknown, carol");
    expect(dmTitle([], "Unknown", "Nobody")).toBe("Nobody");
  });
});
