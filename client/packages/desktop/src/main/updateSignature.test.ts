import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { signatureVerdict } from "./updateSignature.ts";

const subject = "CN=BitCrafters LLC, O=BitCrafters LLC, L=Denver, S=Colorado, C=US";
const checked = (status: unknown, signer: unknown = subject) =>
  JSON.stringify({ status, subject: signer });

void describe("signatureVerdict", () => {
  void it("installs a valid signature from a publisher named by common name or in full", () => {
    assert.equal(signatureVerdict(checked(0), ["BitCrafters LLC"]), null);
    assert.equal(signatureVerdict(checked(0), ["Someone Else", "BitCrafters LLC"]), null);
    assert.equal(signatureVerdict(checked(0), [subject]), null);
    assert.equal(signatureVerdict(checked(0), ["CN=BitCrafters LLC, C=US"]), null);
  });

  void it("refuses another signer, however alike its name", () => {
    for (const publishers of [
      ["BitCrafters"],
      ["bitcrafters llc"],
      ["CN=BitCrafters LLC, C=GB"],
      ["CN=BitCrafters LLC, O=Someone Else"],
      [],
    ]) {
      assert.notEqual(signatureVerdict(checked(0), publishers), null, JSON.stringify(publishers));
    }
    assert.notEqual(
      signatureVerdict(checked(0, "CN=BitCrafters LLC Ltd, C=US"), ["BitCrafters LLC"]),
      null,
    );
  });

  void it("refuses a signature that is not valid, or a check it cannot read", () => {
    for (const status of [1, 2, 3, 4, 5, 6, 7, "0", null]) {
      assert.notEqual(signatureVerdict(checked(status), ["BitCrafters LLC"]), null);
    }
    assert.notEqual(signatureVerdict(checked(0, null), ["BitCrafters LLC"]), null);
    for (const output of ["", "not json", "null", "{}", JSON.stringify({ status: 0 })]) {
      assert.notEqual(signatureVerdict(output, ["BitCrafters LLC"]), null, output);
    }
  });
});
