import { describe, expect, it } from "vitest";
import { CLIENT_PROTOCOL, commonVersion, supports } from "../src/protocol";
import { RecordStore } from "../src/store";
import type { ServerEvent } from "../src/events";

describe("protocol", () => {
  it("speaks the newest version both sides know, or none", () => {
    expect(commonVersion({ version: 3, minimum: 1 }, { version: 5, minimum: 2 })).toBe(3);
    expect(commonVersion({ version: 2, minimum: 1 }, { version: 4, minimum: 3 })).toBeNull();
    expect(commonVersion(CLIENT_PROTOCOL, { version: 1, minimum: 1 })).toBe(1);
    expect(
      supports({ version: 1, minimum: 1, capabilities: ["org.example.x"] }, "org.example.x"),
    ).toBe(true);
    expect(supports({ version: 1, minimum: 1 }, "org.example.x")).toBe(false);
  });

  it("a store ignores events and fields a newer server sends that it does not know", () => {
    const store = new RecordStore({ now: () => 0 });
    const unknown = [
      { serverEvent: "hologram", type: "create", id: "x" },
      { serverEvent: "user", type: "teleport", id: "x" },
      {
        serverEvent: "community",
        type: "create",
        id: "c",
        name: "C",
        icon: null,
        owner: "u",
        aura: 9,
      },
    ] as unknown as ServerEvent[];
    for (const event of unknown) {
      expect(() => {
        store.applyEvent(event);
      }).not.toThrow();
    }
  });
});
