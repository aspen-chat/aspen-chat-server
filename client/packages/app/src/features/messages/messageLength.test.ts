import { describe, expect, it } from "vitest";
import { MESSAGE_MAX_CHARS, messageLength } from "@/features/messages/messageLength";

describe("message length", () => {
  it("counts characters as the server does, not UTF-16 units", () => {
    expect(MESSAGE_MAX_CHARS).toBe(10_000);
    expect(messageLength("")).toBe(0);
    expect(messageLength("hello")).toBe(5);
    expect(messageLength("😀")).toBe(1);
    expect(messageLength("e\u0301")).toBe(2);
    expect(messageLength("中文")).toBe(2);
  });
});
