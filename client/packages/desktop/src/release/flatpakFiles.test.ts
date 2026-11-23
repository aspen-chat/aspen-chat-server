import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { flatpakref, flatpakrepo } from "./flatpakFiles.ts";

const repository = {
  url: "https://downloads.example/flatpak/repo/",
  homepage: "https://example.org",
  key: Buffer.from([0x99, 0x01, 0xff]),
};

void describe("flatpakrepo", () => {
  void it("names the repository and carries its key in base64", () => {
    assert.equal(
      flatpakrepo(repository),
      [
        "[Flatpak Repo]",
        "Title=Aspen",
        "Url=https://downloads.example/flatpak/repo/",
        "Homepage=https://example.org",
        "Comment=Aspen, the community chat app",
        "GPGKey=mQH/",
        "",
      ].join("\n"),
    );
  });

  void it("refuses a value that would end its line early", () => {
    assert.throws(() => flatpakrepo({ ...repository, url: "https://a/\nGPGKey=x" }), /line break/);
  });
});

void describe("flatpakref", () => {
  void it("installs the track's branch, finding the runtime on Flathub", () => {
    const text = flatpakref(repository, "beta");
    assert.match(text, /^\[Flatpak Ref\]\nName=com\.aspen_chat\.client\nBranch=beta\n/);
    assert.match(text, /\nRuntimeRepo=https:\/\/dl\.flathub\.org\/repo\/flathub\.flatpakrepo\n/);
    assert.match(text, /\nSuggestRemoteName=aspen\n/);
    assert.match(text, /\nGPGKey=mQH\/\n$/);
    assert.match(flatpakref(repository, "stable"), /\nBranch=stable\nTitle=Aspen\n/);
  });
});
