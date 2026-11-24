import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { archives, contentType } from "./storage.ts";

void describe("sources.json", () => {
  void it("names archives the bucket knows how to serve, each once", () => {
    const files = archives().map((archive) => archive.file);
    assert.ok(files.length > 0);
    assert.equal(new Set(files).size, files.length);
    for (const file of files) {
      assert.doesNotThrow(() => contentType(file), file);
    }
  });
});
