import { describe, expect, it } from "vitest";
import { RecordStore, manaAt, manaDrawnAt, manaFullAt, type ChannelMana } from "../src";

const at = Date.parse("2026-10-18T12:00:00Z");
// 100 mana at most, 10 a second: 2,000 thousandths a 200 ms tick; at -10 in debt.
const busy: ChannelMana = { level: -10_000, max: 100, recharge: 10, at, queued: true };

function id(n: number): string {
  return `0190f0a0-0000-7000-8000-${n.toString(16).padStart(12, "0")}`;
}

describe("channel mana", () => {
  it("recharges in whole ticks, up to its maximum", () => {
    expect(manaAt(busy, at)).toBe(-10_000);
    expect(manaAt(busy, at + 199)).toBe(-10_000);
    expect(manaAt(busy, at + 200)).toBe(-8_000);
    expect(manaAt(busy, at + 5_000)).toBe(40_000);
    expect(manaAt(busy, at + 60_000)).toBe(100_000);
    // Before the time told, as a clock running behind reads it, nothing is earned back.
    expect(manaAt(busy, at - 1_000)).toBe(-10_000);
  });

  it("is drawn rising steadily, never ahead of the next tick", () => {
    expect(manaDrawnAt(busy, at + 100)).toBe(-9_000);
    for (const elapsed of [0, 67, 133, 199, 200, 4_321]) {
      const drawn = manaDrawnAt(busy, at + elapsed);
      expect(drawn).toBeGreaterThanOrEqual(manaAt(busy, at + elapsed));
      expect(drawn).toBeLessThanOrEqual(manaAt(busy, at + elapsed + 200));
    }
    expect(manaDrawnAt(busy, at + 60_000)).toBe(100_000);
  });

  it("knows when it is full again", () => {
    expect(manaFullAt(busy)).toBe(at + 11_000);
    expect(manaAt(busy, manaFullAt(busy))).toBe(100_000);
  });

  it("is kept while below full, and forgotten when the connection goes", () => {
    const store = new RecordStore({ now: () => at });
    const channel = id(10);
    store.noteMana(channel, busy);
    expect(store.mana(channel)).toEqual(busy);
    // One told full again is not kept.
    store.noteMana(channel, { ...busy, level: 100_000 });
    expect(store.mana(channel)).toBeUndefined();
    store.noteMana(channel, busy);
    store.forgetMana();
    expect(store.mana(channel)).toBeUndefined();
  });
});

describe("waiting for a channel's mana", () => {
  const held = (n: number, extra: object = {}) => ({
    id: id(n),
    channelId: id(10),
    content: "hello",
    attachments: [],
    echoToParent: false,
    heldAt: "2026-10-18T12:00:00Z",
    waiting: "mana" as const,
    kind: "message" as const,
    ...extra,
  });

  it("lets go of one cancelled on another device", () => {
    const store = new RecordStore();
    store.putHeldMessage(held(1));
    store.applyEvent({ serverEvent: "heldMessageCancelled", held: id(1), channel: id(10) });
    expect(store.heldMessage(id(1))).toBeUndefined();
    // The answer to the request that held it, arriving late, does not bring it back.
    store.putHeldMessage(held(1));
    expect(store.heldMessage(id(1))).toBeUndefined();
  });

  it("lets go of an edit whose message was deleted", () => {
    const store = new RecordStore();
    store.putHeldMessage(held(2, { kind: "edit", target: id(20) }));
    store.applyEvent({ serverEvent: "message", type: "delete", id: id(20) });
    expect(store.heldMessage(id(2))).toBeUndefined();
  });
});

describe("mana frames out of order", () => {
  it("keeps the latest frame by its time, whichever server's arrives last", () => {
    const store = new RecordStore({ now: () => at });
    const channel = id(10);
    store.noteMana(channel, busy);
    // A frame from before the one held, overtaken on its way from another server.
    store.noteMana(channel, { ...busy, level: -5_000, at: at - 1_000 });
    expect(store.mana(channel)).toEqual(busy);
    const later: ChannelMana = { ...busy, level: -2_000, at: at + 1_000 };
    store.noteMana(channel, later);
    expect(store.mana(channel)).toEqual(later);
    // One that said the channel was full stands against an older one that says it was not.
    const other = id(11);
    store.noteMana(other, { ...busy, level: 100_000, at: at - 1_000 });
    expect(store.mana(other)).toBeUndefined();
    store.noteMana(other, { ...busy, at: at - 2_000 });
    expect(store.mana(other)).toBeUndefined();
  });
});
