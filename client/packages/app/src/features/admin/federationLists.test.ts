import type { FederationList } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import { listHint } from "@/features/admin/federationLists";
import { en } from "@/i18n/messages";

describe("listHint", () => {
  it("names a list's subject, direction, and kind", () => {
    expect(listHint(en, "usersEmigrationAllow")).toBe("Users · Going elsewhere · Allow list");
    expect(listHint(en, "botsImmigrationBlock")).toBe("Bots · Coming here · Block list");
    expect(listHint(en, "usersSharedBlock")).toBe("Users · One list for both ways · Block list");
  });

  it("names a list it does not know as the server does, guessing nothing", () => {
    expect(listHint(en, "pluginsImmigrationAllow" as FederationList)).toBe(
      "pluginsImmigrationAllow",
    );
    expect(listHint(en, "usersTransitBlock" as FederationList)).toBe("usersTransitBlock");
  });
});
