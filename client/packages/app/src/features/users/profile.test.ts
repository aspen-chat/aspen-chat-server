import type { User } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { displayNameOf, handleOf, profileForm, profilePatch, statusLine } from "./profile";

const kate: User = {
  id: "u1",
  name: "kate1024",
  icon: null,
  onlineStatus: "online",
  bot: false,
  botPublic: false,
  displayName: "Kate",
  pronouns: "she/her",
  status: { text: "shipping", emoji: "🚀" },
};

describe("profiles", () => {
  it("prefers a display name and falls back to the username", () => {
    expect(displayNameOf(kate)).toBe("Kate");
    expect(displayNameOf({ name: "bob", displayName: "  " })).toBe("bob");
    expect(displayNameOf({ name: "bob" })).toBe("bob");
    expect(statusLine({ text: "away" })).toBe("away");
    expect(statusLine({ text: "away", emoji: "🌴" })).toBe("🌴 away");
  });

  it("builds a merge patch of only what changed, clearing blanks with null", () => {
    const form = profileForm(kate);
    expect(profilePatch(kate, form)).toEqual({});
    expect(profilePatch(kate, { ...form, pronouns: "  ", bio: " new bio " })).toEqual({
      pronouns: null,
      bio: "new bio",
    });
    expect(profilePatch(kate, { ...form, statusEmoji: null })).toEqual({
      status: { text: "shipping" },
    });
    expect(profilePatch(kate, { ...form, statusText: "" })).toEqual({ status: null });
    expect(profilePatch(kate, { ...form, icon: "i1" })).toEqual({ icon: "i1" });
    expect(profilePatch({ ...kate, icon: "i1" }, { ...form, icon: null })).toEqual({ icon: null });
    expect(profilePatch(kate, { ...form, statusText: "lunch", statusEmoji: "🍕" })).toEqual({
      status: { text: "lunch", emoji: "🍕" },
    });
    const plain: User = {
      id: "u2",
      name: "bob",
      icon: null,
      onlineStatus: "offline",
      bot: false,
      botPublic: false,
    };
    expect(profilePatch(plain, { ...profileForm(plain), statusEmoji: "🍕" })).toEqual({});
  });

  it("gives a user of another deployment a handle naming their home", () => {
    expect(handleOf({ name: "kate", homeDomain: null })).toBe("@kate");
    expect(handleOf({ name: "kate", homeDomain: "b.example:8443" })).toBe("@kate@b.example:8443");
  });
});
