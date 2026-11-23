/**
 * Publishes a Windows release to the downloads bucket (`storage.ts`), in the two steps
 * `.github/workflows/ci.yml` runs on each side of making the GitHub Release:
 *
 *   node src/release/publishWindows.ts upload --installer <Aspen-Setup-X.Y.Z.exe> [--prefix test/]
 *   node src/release/publishWindows.ts announce --installer <Aspen-Setup-X.Y.Z.exe> [--prefix test/]
 *
 * `upload` writes the files that never change: the source archives of the copyleft libraries
 * the installer ships (`sources.json`), each kept once, and the installer, which it refuses to
 * write over with another (one with the same SHA-256 is left as it is, so a publish that stopped
 * part way runs again). `announce` then writes what names them, the track's update feed and
 * `releases.json`, last of all, so nobody is pointed at a file not there yet. Both refuse a
 * version no newer than its track's head (`release` in `releases.ts`) before writing anything.
 * The version is the desktop package's own, which the release job has already checked against
 * its tag. One publish runs at a time (the job's concurrency group): `announce` reads
 * `releases.json`, decides, and writes it back.
 */

import { readFileSync, statSync } from "node:fs";
import { basename } from "node:path";
import { parseArgs } from "node:util";
import { FEED_FILES, WINDOWS_X64, feedText, release, type Build } from "./releases.ts";
import {
  IMMUTABLE,
  REPLACED,
  archives,
  currentReleases,
  digest,
  mirrorSources,
  openBucket,
  packageVersion,
} from "./storage.ts";

async function main(): Promise<void> {
  const { positionals, values } = parseArgs({
    allowPositionals: true,
    options: {
      installer: { type: "string" },
      prefix: { type: "string", default: "" },
    },
  });
  const [step] = positionals;
  if ((step !== "upload" && step !== "announce") || positionals.length !== 1) {
    throw new Error("say `upload` or `announce`");
  }
  if (values.installer === undefined) {
    throw new Error("--installer names the signed installer to publish");
  }
  const { bucket, publicUrl: folderUrl } = openBucket(values.prefix);
  const publicUrl = folderUrl(WINDOWS_X64.folder);
  const version = packageVersion();
  const installer = values.installer;
  if (basename(installer) !== basename(WINDOWS_X64.installer(version))) {
    throw new Error(`${installer} is not the installer of version ${version}`);
  }
  const data = readFileSync(installer);
  const installerPath = WINDOWS_X64.installer(version);
  const build: Build = {
    version,
    url: new URL(installerPath, publicUrl).href,
    sha256: digest("sha256", data, "hex"),
    sha512: digest("sha512", data, "base64"),
    size: statSync(installer).size,
    released: new Date().toISOString(),
  };
  // Refused here, before anything is written, as well as in `announce`.
  const { releases, tracks } = release(currentReleases(bucket), WINDOWS_X64.key, build);
  const key = `${WINDOWS_X64.folder}/${installerPath}`;

  if (step === "upload") {
    await mirrorSources(bucket);
    if (bucket.size(key) === null) {
      bucket.write(key, installer, IMMUTABLE, { sha256: build.sha256 });
    } else if (bucket.metadata(key, "sha256") !== build.sha256) {
      throw new Error(`${bucket.key(key)} is already published, as another build; a version is released once`);
    }
    return;
  }

  if (bucket.size(key) === null || bucket.metadata(key, "sha256") !== build.sha256) {
    throw new Error(`${bucket.key(key)} is not the installer given; run \`upload\` first`);
  }
  for (const archive of archives()) {
    if (bucket.size(`sources/${archive.file}`) === null) {
      throw new Error(`sources/${archive.file} is missing; run \`upload\` first`);
    }
  }
  for (const track of tracks) {
    bucket.writeText(
      `${WINDOWS_X64.folder}/${FEED_FILES[track]}`,
      feedText(build, installerPath),
      REPLACED,
    );
  }
  bucket.writeText("releases.json", `${JSON.stringify(releases, null, 2)}\n`, REPLACED);
  console.log(`published ${version} to the ${tracks.join(" and ")} track${tracks.length > 1 ? "s" : ""}`);
}

await main();
