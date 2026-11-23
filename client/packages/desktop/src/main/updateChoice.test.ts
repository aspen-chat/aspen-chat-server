import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { defaultChoice, mayInstall, parseChoice } from "./updateChoice.ts";

void describe("parseChoice", () => {
  void it("takes the three choices alone", () => {
    assert.equal(parseChoice("stable"), "stable");
    assert.equal(parseChoice("beta"), "beta");
    assert.equal(parseChoice("off"), "off");
    for (const refused of ["latest", "", null, undefined, 1, { choice: "beta" }]) {
      assert.equal(parseChoice(refused), null);
    }
  });
});

void describe("defaultChoice", () => {
  void it("follows the track the installed build came from", () => {
    assert.equal(defaultChoice("0.2.0"), "stable");
    assert.equal(defaultChoice("0.3.0-beta.1"), "beta");
    assert.equal(defaultChoice("0.3.0-dev"), "stable");
  });
});

void describe("mayInstall", () => {
  void it("installs a beta only for someone following beta", () => {
    assert.equal(mayInstall("beta", "0.3.0-beta.1"), true);
    assert.equal(mayInstall("beta", "0.3.0"), true);
    assert.equal(mayInstall("stable", "0.3.0"), true);
    assert.equal(mayInstall("stable", "0.3.0-beta.1"), false);
    assert.equal(mayInstall("off", "0.3.0"), false);
    assert.equal(mayInstall("beta", "not a version"), false);
  });
});
