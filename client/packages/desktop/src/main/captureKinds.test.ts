import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  NOTHING_OFFERED,
  TEST_PATTERN_KIND,
  audioOffered,
  offeredKinds,
  startOffered,
} from "./captureKinds.ts";

const offered = offeredKinds({
  kinds: [
    { kind: "game_capture", audio: { kind: "wasapi_process_output_capture" } },
    { kind: "window_capture", audio: null },
  ],
  applicationAudio: { kind: "pipewire_application_capture" },
  testMedia: null,
});

void describe("startOffered", () => {
  void it("starts what was listed", () => {
    assert.equal(startOffered(offered, { kind: "game_capture" }), true);
    assert.equal(
      startOffered(offered, {
        kind: "game_capture",
        audio: { kind: "wasapi_process_output_capture" },
      }),
      true,
    );
    assert.equal(audioOffered(offered, { kind: "pipewire_application_capture" }), true);
  });

  void it("refuses what was not", () => {
    for (const kind of ["browser_source", "image_source", TEST_PATTERN_KIND, "dshow_input"]) {
      assert.equal(startOffered(offered, { kind }), false, kind);
    }
    assert.equal(
      startOffered(offered, { kind: "game_capture", audio: { kind: "wasapi_input_capture" } }),
      false,
    );
    assert.equal(audioOffered(offered, { kind: "wasapi_input_capture" }), false);
    assert.equal(startOffered(NOTHING_OFFERED, { kind: "game_capture" }), false);
    assert.equal(audioOffered(NOTHING_OFFERED, { kind: "" }), false);
  });

  void it("plays the test pattern only from the file offered", () => {
    const testing = offeredKinds({ kinds: [], applicationAudio: null, testMedia: "/clip.mp4" });
    const settings = (file: string) => JSON.stringify({ is_local_file: true, local_file: file });
    assert.equal(
      startOffered(testing, {
        kind: TEST_PATTERN_KIND,
        settings: settings("/clip.mp4"),
        audio: { kind: "" },
      }),
      true,
    );
    assert.equal(
      startOffered(testing, { kind: TEST_PATTERN_KIND, settings: settings("/etc/passwd") }),
      false,
    );
    assert.equal(startOffered(testing, { kind: TEST_PATTERN_KIND, settings: "{" }), false);
  });
});
