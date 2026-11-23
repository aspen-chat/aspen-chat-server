import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  EMPTY_RELEASES,
  WINDOWS_X64,
  compareVersions,
  feedText,
  parseReleases,
  parseVersion,
  release,
  versionOfTag,
  type Build,
  type Version,
} from "./releases.ts";

function version(text: string): Version {
  const parsed = parseVersion(text);
  assert.notEqual(parsed, null, text);
  return parsed as Version;
}

function build(text: string): Build {
  return {
    version: text,
    url: `https://downloads.example/windows/x64/${WINDOWS_X64.installer(text)}`,
    sha256: "00",
    sha512: "AA==",
    size: 1,
    released: "2026-11-22T00:00:00.000Z",
  };
}

void describe("parseVersion", () => {
  void it("takes stable and beta versions alone", () => {
    assert.deepEqual(parseVersion("0.2.0"), { major: 0, minor: 2, patch: 0, beta: null });
    assert.deepEqual(parseVersion("1.10.3-beta.12"), { major: 1, minor: 10, patch: 3, beta: 12 });
    for (const refused of [
      "",
      "1.2",
      "1.2.3.4",
      "01.2.3",
      "1.2.3-beta",
      "1.2.3-beta.01",
      "1.2.3-rc.1",
      "1.2.3+build",
      "v1.2.3",
      " 1.2.3",
      "1.2.3\n",
      "1.2.3-beta.1-beta.2",
    ]) {
      assert.equal(parseVersion(refused), null, JSON.stringify(refused));
    }
  });

  void it("reads a tag's version", () => {
    assert.deepEqual(versionOfTag("client-v0.2.0-beta.1"), version("0.2.0-beta.1"));
    assert.equal(versionOfTag("v0.2.0"), null);
    assert.equal(versionOfTag("client-v0.2"), null);
  });
});

void describe("compareVersions", () => {
  void it("orders as semver does, a beta before its stable version", () => {
    const ordered = [
      "0.1.9",
      "0.2.0-beta.1",
      "0.2.0-beta.2",
      "0.2.0-beta.10",
      "0.2.0",
      "0.2.1",
      "0.10.0",
      "1.0.0-beta.0",
      "1.0.0",
    ].map(version);
    for (let i = 0; i < ordered.length; i += 1) {
      for (let j = 0; j < ordered.length; j += 1) {
        const a = ordered[i] as Version;
        const b = ordered[j] as Version;
        assert.equal(Math.sign(compareVersions(a, b)), Math.sign(i - j));
      }
    }
  });
});

void describe("release", () => {
  const key = WINDOWS_X64.key;

  void it("puts a first stable build at the head of both tracks", () => {
    const { releases, tracks } = release(EMPTY_RELEASES, key, build("0.2.0"));
    assert.deepEqual(tracks, ["stable", "beta"]);
    assert.equal(releases.builds[key]?.stable?.version, "0.2.0");
    assert.equal(releases.builds[key]?.beta?.version, "0.2.0");
  });

  void it("puts a beta on the beta track alone", () => {
    const first = release(EMPTY_RELEASES, key, build("0.2.0")).releases;
    const { releases, tracks } = release(first, key, build("0.3.0-beta.1"));
    assert.deepEqual(tracks, ["beta"]);
    assert.equal(releases.builds[key]?.stable?.version, "0.2.0");
    assert.equal(releases.builds[key]?.beta?.version, "0.3.0-beta.1");
  });

  void it("leaves a newer beta where it is when an older stable version is released", () => {
    let releases = release(EMPTY_RELEASES, key, build("0.2.0")).releases;
    releases = release(releases, key, build("0.4.0-beta.1")).releases;
    const next = release(releases, key, build("0.3.0"));
    assert.deepEqual(next.tracks, ["stable"]);
    assert.equal(next.releases.builds[key]?.beta?.version, "0.4.0-beta.1");
  });

  void it("moves beta testers on to the stable version their beta became", () => {
    let releases = release(EMPTY_RELEASES, key, build("0.3.0-beta.2")).releases;
    const next = release(releases, key, build("0.3.0"));
    assert.deepEqual(next.tracks, ["stable", "beta"]);
    releases = next.releases;
    assert.equal(releases.builds[key]?.beta?.version, "0.3.0");
  });

  void it("refuses a version no newer than its track's head", () => {
    let releases = release(EMPTY_RELEASES, key, build("0.3.0")).releases;
    releases = release(releases, key, build("0.4.0-beta.2")).releases;
    for (const refused of ["0.3.0", "0.2.9", "0.4.0-beta.2", "0.4.0-beta.1", "0.3.0-beta.9"]) {
      assert.throws(() => release(releases, key, build(refused)), /no newer/, refused);
    }
    assert.throws(() => release(releases, key, build("0.4")), /not a release version/);
  });

  void it("keeps other platforms' builds", () => {
    const other = { builds: { "macos-arm64": { stable: build("9.0.0"), beta: null } } };
    const { releases } = release(other, key, build("0.2.0"));
    assert.equal(releases.builds["macos-arm64"]?.stable?.version, "9.0.0");
  });
});

void describe("parseReleases", () => {
  void it("reads back what it wrote", () => {
    const { releases } = release(EMPTY_RELEASES, WINDOWS_X64.key, build("0.2.0"));
    assert.deepEqual(parseReleases(JSON.parse(JSON.stringify(releases))), releases);
  });

  void it("refuses a file it did not write", () => {
    for (const refused of [
      null,
      [],
      {},
      { builds: [] },
      { builds: { "windows-x64": "0.2.0" } },
      { builds: { "windows-x64": { stable: { version: "0.2.0" } } } },
      { builds: { "windows-x64": { stable: { ...build("0.2.0"), version: "latest" } } } },
    ]) {
      assert.throws(() => parseReleases(refused), JSON.stringify(refused));
    }
  });
});

void describe("feedText", () => {
  void it("names the installer relative to the feed, quoting every string", () => {
    const text = feedText(
      { ...build("0.2.0-beta.1"), sha512: "a+b/c==" },
      WINDOWS_X64.installer("0.2.0-beta.1"),
    );
    assert.equal(
      text,
      [
        "version: '0.2.0-beta.1'",
        "files:",
        "  - url: '0.2.0-beta.1/Aspen-Setup-0.2.0-beta.1.exe'",
        "    sha512: 'a+b/c=='",
        "    size: 1",
        "path: '0.2.0-beta.1/Aspen-Setup-0.2.0-beta.1.exe'",
        "sha512: 'a+b/c=='",
        "releaseDate: '2026-11-22T00:00:00.000Z'",
        "",
      ].join("\n"),
    );
  });
});
