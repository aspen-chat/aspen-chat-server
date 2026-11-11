import { describe, expect, it } from "vitest";
import { actorKindOf } from "./moderationActor";

const someone = "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

describe("actorKindOf", () => {
  it("is what the deployment says, whatever the action", () => {
    expect(actorKindOf({ actor: null, actorKind: "operator", action: "banUser" })).toBe("operator");
    expect(
      actorKindOf({ actor: null, actorKind: "deletedAccount", action: "purgeAttachment" }),
    ).toBe("deletedAccount");
    expect(actorKindOf({ actor: someone, actorKind: "user", action: "banUser" })).toBe("user");
  });

  it("is read from the actor and the action for a deployment that does not say", () => {
    expect(actorKindOf({ actor: someone, action: "banUser" })).toBe("user");
    expect(actorKindOf({ actor: null, action: "purgeAttachment" })).toBe("operator");
    expect(actorKindOf({ actor: null, action: "banUser" })).toBe("deletedAccount");
  });
});
