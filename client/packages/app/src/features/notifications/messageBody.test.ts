import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  messageBody,
  ReadBudget,
  SERVER_TEXT_WAIT_MS,
  TEXT_READ_EVERY_MS,
  TEXT_READS_AT_ONCE,
} from "./messageBody";

describe("messageBody", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("is the deployment's wording when the read answers one", async () => {
    await expect(messageBody(() => Promise.resolve("hi @Bob :party:"), "own")).resolves.toBe(
      "hi @Bob :party:",
    );
    expect(vi.getTimerCount()).toBe(0);
  });

  it("is the app's own wording when the read fails", async () => {
    await expect(messageBody(() => Promise.reject(new Error("rate limited")), "own")).resolves.toBe(
      "own",
    );
  });

  it("is the app's own wording when the deployment writes none", async () => {
    await expect(messageBody(() => Promise.resolve(undefined), "own")).resolves.toBe("own");
    await expect(messageBody(() => Promise.resolve(""), "own")).resolves.toBe("own");
  });

  it("is the app's own wording when the read takes too long", async () => {
    let answer: (text: string) => void = () => undefined;
    const body = messageBody(
      () =>
        new Promise<string>((resolve) => {
          answer = resolve;
        }),
      "own",
    );
    let settled: string | undefined;
    void body.then((text) => {
      settled = text;
    });
    await vi.advanceTimersByTimeAsync(SERVER_TEXT_WAIT_MS - 1);
    expect(settled).toBeUndefined();
    await vi.advanceTimersByTimeAsync(1);
    expect(settled).toBe("own");
    answer("late");
    await expect(body).resolves.toBe("own");
  });
});

describe("ReadBudget", () => {
  it("lets a burst read and then one more each interval", () => {
    const budget = new ReadBudget();
    const start = 1_000_000;
    for (let i = 0; i < TEXT_READS_AT_ONCE; i += 1) {
      expect(budget.take(start + i)).toBe(true);
    }
    expect(budget.take(start + TEXT_READS_AT_ONCE)).toBe(false);
    expect(budget.take(start + TEXT_READ_EVERY_MS - 1)).toBe(false);
    expect(budget.take(start + TEXT_READ_EVERY_MS)).toBe(true);
    expect(budget.take(start + TEXT_READ_EVERY_MS + 1)).toBe(false);
  });

  it("never holds more than a burst, however long it rests", () => {
    const budget = new ReadBudget();
    expect(budget.take(0)).toBe(true);
    const later = 1000 * TEXT_READ_EVERY_MS;
    let taken = 0;
    while (budget.take(later)) {
      taken += 1;
    }
    expect(taken).toBe(TEXT_READS_AT_ONCE);
  });

  it("keeps a steady stream of messages to one read each interval", () => {
    const budget = new ReadBudget();
    let taken = 0;
    // Twenty messages a second for a minute.
    for (let at = 0; at < 60_000; at += 50) {
      if (budget.take(at)) {
        taken += 1;
      }
    }
    expect(taken).toBe(TEXT_READS_AT_ONCE + Math.floor(59_950 / TEXT_READ_EVERY_MS));
  });
});
