import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { lockOf } from "./updatePolicy.ts";

const off = { type: "Int32", value: "1" };
const on = { type: "Int32", value: "0" };
const output = (machine: unknown, user: unknown) => JSON.stringify({ machine, user });

void describe("lockOf", () => {
  void it("leaves updating on with neither value, or with each the DWORD 0", () => {
    assert.equal(lockOf(output(null, null)), null);
    assert.equal(lockOf(output(on, on)), null);
  });

  void it("names who turned it off, the administrator first", () => {
    assert.equal(lockOf(output(off, null)), "administrator");
    assert.equal(lockOf(output(off, off)), "administrator");
    assert.equal(lockOf(output(null, off)), "installer");
    assert.equal(lockOf(output(on, off)), "installer");
  });

  void it("turns it off for any value but the DWORD 0", () => {
    for (const value of [
      { type: "String", value: "0" },
      { type: "Int64", value: "0" },
      { type: "Int32", value: "-1" },
      { type: "String[]", value: "" },
    ]) {
      assert.equal(lockOf(output(value, null)), "administrator", JSON.stringify(value));
    }
  });

  void it("locks on output it cannot read", () => {
    for (const refused of [
      "",
      "not json",
      "null",
      "[]",
      JSON.stringify({ machine: null }),
      output("1", null),
      output(null, { type: 1, value: "1" }),
    ]) {
      assert.equal(lockOf(refused), "administrator", refused);
    }
  });
});
