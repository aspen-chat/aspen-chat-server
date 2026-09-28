import { describe, expect, it } from "vitest";
import spec from "../../../../../spec/moderation_actions.json";
import { en } from "@/i18n/messages";

describe("messages", () => {
  it("names every action the moderation log records", () => {
    expect(Object.keys(en.admin.moderationActions).sort()).toEqual([...spec.actions].sort());
  });
});
