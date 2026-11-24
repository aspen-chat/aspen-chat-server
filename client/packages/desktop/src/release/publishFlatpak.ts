/**
 * Publishes a Flatpak release to the downloads bucket (`storage.ts`): a new signed commit of the
 * app in Aspen's Flatpak repository (`flatpak/repo/`), on the branch of each track it heads, in
 * the two steps `.github/workflows/ci.yml` runs on each side of making the GitHub Release:
 *
 *   node src/release/publishFlatpak.ts upload --bundle <Aspen-X.Y.Z.flatpak> --work <dir> \
 *     --timestamp <ISO 8601> [--prefix test/] [--public-key <file>]
 *   node src/release/publishFlatpak.ts announce (the same)
 *
 * The bundle is `scripts/flatpak.mjs`'s, unsigned. `upload` reads the published repository into
 * `--work`, commits the bundle's app to it (`flatpak build-commit-from`), signed, with the head
 * before it as its parent, has Flatpak make the static deltas and the AppStream data and prune
 * history beyond `HISTORY` commits, and makes a signed bundle of the new commit that names the
 * repository, so a bundle installed by hand updates from it. It then writes the files that never
 * change: the source archives (`sources.json`), the repository's new objects and deltas, and the
 * bundle. `announce`, with the same `--work`, writes what names them last: the repository's refs
 * and summary, the `.flatpakrepo` and `.flatpakref` files, and `releases.json`, and then removes
 * what the repository no longer holds.
 *
 * The commit's timestamp is `--timestamp` (the release commit's), so running `upload` again from
 * the same published state makes the same commit, and the bundle already uploaded for it is kept.
 *
 * Signing takes `ASPEN_FLATPAK_GPG_KEY`, the key's fingerprint, from the keyring
 * `ASPEN_FLATPAK_GPG_HOMEDIR` names (or the default one); the key must be the one `--public-key`
 * holds (`flatpak/aspen-packages.asc`), which people's machines check the repository against.
 */

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { parseArgs } from "node:util";
import {
  APP_ID,
  FLATPAKREF_FILES,
  RUNTIME_REPO,
  flatpakref,
  flatpakrepo,
  type Repository,
} from "./flatpakFiles.ts";
import { LINUX_X64_FLATPAK, release, type Build, type Track } from "./releases.ts";
import {
  IMMUTABLE,
  REPLACED,
  archives,
  currentReleases,
  desktop,
  digest,
  mirrorSources,
  openBucket,
  packageVersion,
} from "./storage.ts";

/** How many commits of each branch the repository keeps, the head among them. */
const HISTORY = 10;
const ARCH = "x86_64";
const REPOSITORY = "flatpak/repo";
const HOMEPAGE = "https://aspen-chat.com";
/** The repository's mutable files: Flatpak re-reads them on every update, so none is cached. */
const NOT_CACHED = "no-cache";
/** What a local repository holds that is never published: work in progress, its lock, and state. */
const UNPUBLISHED = ["tmp/*", "state/*", "extensions/*", ".lock"];

function run(command: string, args: string[]): string {
  const result = spawnSync(command, args, { encoding: "utf8", maxBuffer: 64 * 1024 * 1024 });
  if (result.error !== undefined) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed (${String(result.status)}):\n${result.stderr}`);
  }
  return result.stdout;
}

function ref(track: Track): string {
  return `app/${APP_ID}/${ARCH}/${track}`;
}

/**
 * The public key's fingerprint and its binary packets, read by GnuPG without importing it
 * anywhere.
 */
function publicKey(file: string): { fingerprint: string; packets: Buffer } {
  if (!existsSync(file)) {
    throw new Error(`${file} is missing: it holds the repository's public key, which people's machines check it against`);
  }
  const listed = run("gpg", ["--with-colons", "--import-options", "show-only", "--import", file]);
  const fingerprint = listed
    .split("\n")
    .find((line) => line.startsWith("fpr:"))
    ?.split(":")[9];
  if (fingerprint === undefined || fingerprint === "") {
    throw new Error(`${file} holds no OpenPGP key`);
  }
  const packets = spawnSync("gpg", ["--dearmor"], { input: readFileSync(file) });
  if (packets.status !== 0) {
    throw new Error(`${file} could not be read: ${packets.stderr.toString()}`);
  }
  return { fingerprint, packets: packets.stdout };
}

/** The signing options, once the key they name is known to be the published one's. */
function signing(published: string): string[] {
  const key = process.env.ASPEN_FLATPAK_GPG_KEY;
  if (key === undefined || key === "") {
    throw new Error("ASPEN_FLATPAK_GPG_KEY names the key the repository is signed with");
  }
  const homedir = process.env.ASPEN_FLATPAK_GPG_HOMEDIR;
  const homedirArgs = homedir === undefined ? [] : ["--homedir", homedir];
  const listed = run("gpg", [...homedirArgs, "--with-colons", "--list-secret-keys", key]);
  const primary = listed
    .split("\n")
    .find((line) => line.startsWith("fpr:"))
    ?.split(":")[9];
  if (primary !== published) {
    throw new Error(
      `${key} is not the key the repository is published with (${published}); people's machines would refuse what it signs`,
    );
  }
  return [`--gpg-sign=${key}`, ...(homedir === undefined ? [] : [`--gpg-homedir=${homedir}`])];
}

/** The checksum of the commit `ref` names in `repository`, or `null` when it names none. */
function head(repository: string, name: string): string | null {
  const listed = run("ostree", [`--repo=${repository}`, "refs"]).split("\n");
  return listed.includes(name)
    ? run("ostree", [`--repo=${repository}`, "rev-parse", name]).trim()
    : null;
}

async function main(): Promise<void> {
  const { positionals, values } = parseArgs({
    allowPositionals: true,
    options: {
      bundle: { type: "string" },
      work: { type: "string" },
      timestamp: { type: "string" },
      prefix: { type: "string", default: "" },
      "public-key": { type: "string", default: join(desktop, "flatpak", "aspen-packages.asc") },
    },
  });
  const [step] = positionals;
  if ((step !== "upload" && step !== "announce") || positionals.length !== 1) {
    throw new Error("say `upload` or `announce`");
  }
  const { bundle: given, work, timestamp } = values;
  if (given === undefined || work === undefined || timestamp === undefined) {
    throw new Error("--bundle, --work, and --timestamp are all needed");
  }
  if (Number.isNaN(Date.parse(timestamp))) {
    throw new Error(`--timestamp ${timestamp} is not an instant`);
  }
  const version = packageVersion();
  if (basename(given) !== basename(LINUX_X64_FLATPAK.bundle(version))) {
    throw new Error(`${given} is not the bundle of version ${version}`);
  }

  const { bucket, publicUrl } = openBucket(values.prefix);
  const repositoryUrl = publicUrl(REPOSITORY).href;
  const key = publicKey(values["public-key"]);
  const sign = signing(key.fingerprint);

  const bundlePath = LINUX_X64_FLATPAK.bundle(version);
  const bundleKey = `${LINUX_X64_FLATPAK.folder}/${bundlePath}`;
  const repository = join(work, "repo");
  const signed = join(work, basename(bundlePath));
  const placeholder: Build = {
    version,
    url: new URL(bundlePath, publicUrl(LINUX_X64_FLATPAK.folder)).href,
    sha256: "",
    sha512: "",
    size: 0,
    released: new Date().toISOString(),
  };
  // Refused here, before anything is read or written, as well as below.
  const { tracks } = release(currentReleases(bucket), LINUX_X64_FLATPAK.key, placeholder);
  const own = tracks[0];
  if (own === undefined) {
    throw new Error(`${version} heads no track`);
  }

  if (step === "upload") {
    await mirrorSources(bucket);
    mkdirSync(work, { recursive: true });
    // A fresh repository first, then the published one's files over it: the bucket keeps no
    // empty folders, and OSTree needs every one of its own.
    if (existsSync(repository)) {
      throw new Error(`${repository} exists; give --work a folder of its own for each release`);
    }
    run("ostree", ["init", "--mode=archive-z2", `--repo=${repository}`]);
    bucket.download(REPOSITORY, repository);
    const imported = join(work, "imported");
    run("ostree", ["init", "--mode=archive-z2", `--repo=${imported}`]);
    run("flatpak", ["build-import-bundle", "--no-update-summary", imported, given]);
    const source = run("ostree", [`--repo=${imported}`, "refs"])
      .split("\n")
      .find((line) => line.startsWith(`app/${APP_ID}/${ARCH}/`));
    if (source === undefined) {
      throw new Error(`${given} holds no build of ${APP_ID} for ${ARCH}`);
    }
    for (const track of tracks) {
      run("flatpak", [
        "build-commit-from",
        `--src-repo=${imported}`,
        `--src-ref=${source}`,
        `--subject=Aspen ${version}`,
        `--timestamp=${timestamp}`,
        "--no-update-summary",
        ...sign,
        repository,
        ref(track),
      ]);
    }
    run("flatpak", [
      "build-update-repo",
      "--title=Aspen",
      `--homepage=${HOMEPAGE}`,
      "--default-branch=stable",
      "--generate-static-deltas",
      "--prune",
      `--prune-depth=${String(HISTORY - 1)}`,
      ...sign,
      repository,
    ]);
    const keyFile = join(work, "aspen-packages.gpg");
    writeFileSync(keyFile, key.packets);
    run("flatpak", [
      "build-bundle",
      `--repo-url=${repositoryUrl}`,
      `--runtime-repo=${RUNTIME_REPO}`,
      `--gpg-keys=${keyFile}`,
      ...sign,
      repository,
      signed,
      APP_ID,
      own,
    ]);
    bucket.upload(repository, REPOSITORY, ["objects/*", "deltas/*"], "named", IMMUTABLE);
    const commit = head(repository, ref(own));
    if (commit === null) {
      throw new Error(`${ref(own)} was not committed`);
    }
    if (bucket.size(bundleKey) === null) {
      bucket.write(bundleKey, signed, IMMUTABLE, {
        commit,
        sha256: digest("sha256", readFileSync(signed), "hex"),
      });
    } else if (bucket.metadata(bundleKey, "commit") !== commit) {
      throw new Error(`${bucket.key(bundleKey)} is already published, as another build; a version is released once`);
    }
    return;
  }

  const commit = existsSync(repository) ? head(repository, ref(own)) : null;
  if (commit === null || bucket.metadata(bundleKey, "commit") !== commit) {
    throw new Error(`${bucket.key(bundleKey)} is not the bundle of this commit; run \`upload\` first, with the same --work`);
  }
  for (const archive of archives()) {
    if (bucket.size(`sources/${archive.file}`) === null) {
      throw new Error(`sources/${archive.file} is missing; run \`upload\` first`);
    }
  }
  const published = readFileSync(signed);
  const build: Build = {
    ...placeholder,
    sha256: digest("sha256", published, "hex"),
    sha512: digest("sha512", published, "base64"),
    size: statSync(signed).size,
  };
  const { releases } = release(currentReleases(bucket), LINUX_X64_FLATPAK.key, build);

  // What names the objects goes last, the summary last of all, which Flatpak reads first.
  bucket.upload(
    repository,
    REPOSITORY,
    ["config", "refs/*", "delta-indexes/*", "summaries/*"],
    "replaced",
    NOT_CACHED,
  );
  for (const file of ["summary.sig", "summary", "summary.idx.sig", "summary.idx"]) {
    if (existsSync(join(repository, file))) {
      bucket.upload(repository, REPOSITORY, [file], "replaced", NOT_CACHED);
    }
  }
  const pointers: Repository = { url: repositoryUrl, homepage: HOMEPAGE, key: key.packets };
  bucket.writeText("flatpak/aspen.flatpakrepo", flatpakrepo(pointers), NOT_CACHED);
  for (const track of ["stable", "beta"] as const) {
    bucket.writeText(`flatpak/${FLATPAKREF_FILES[track]}`, flatpakref(pointers, track), NOT_CACHED);
  }
  bucket.writeText("releases.json", `${JSON.stringify(releases, null, 2)}\n`, REPLACED);
  bucket.removeAbsent(repository, REPOSITORY, UNPUBLISHED);
  console.log(
    `published ${version} to the ${tracks.join(" and ")} branch${tracks.length > 1 ? "es" : ""}`,
  );
}

await main();
