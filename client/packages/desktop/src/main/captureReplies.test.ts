import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { parseReply } from "./captureReplies.ts";

void describe("parseReply", () => {
  void it("reads each kind of reply", () => {
    assert.deepEqual(parseReply('{"type":"done","id":4}'), { type: "done", id: 4 });
    assert.deepEqual(parseReply('{"type":"failed","id":5,"reason":"no"}'), {
      type: "failed",
      id: 5,
      reason: "no",
    });
    assert.deepEqual(parseReply('{"type":"kinds","id":6,"kinds":[],"applicationAudio":null}'), {
      type: "kinds",
      id: 6,
      kinds: [],
      applicationAudio: null,
    });
  });

  void it("takes a line that is not a reply for none", () => {
    for (const line of [
      "info: ---------------------------------",
      "",
      "7",
      "null",
      '"done"',
      '{"type":"done"}',
      '{"type":"done","id":"4"}',
      '{"type":"progress","id":4}',
      '{"id":4}',
    ]) {
      assert.equal(parseReply(line), null, line);
    }
  });
});
