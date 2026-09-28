import { describe, expect, it } from "vitest";
import { suggestedPermissions } from "@/features/bots/botLink";

describe("suggestedPermissions", () => {
  it("keeps known names once, in the listed order", () => {
    expect(suggestedPermissions("sendMessages,manageRoles,nonsense,sendMessages")).toEqual([
      "manageRoles",
      "sendMessages",
    ]);
  });

  it("suggests nothing without the parameter", () => {
    expect(suggestedPermissions(undefined)).toEqual([]);
    expect(suggestedPermissions("")).toEqual([]);
  });
});
