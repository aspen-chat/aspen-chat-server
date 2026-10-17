// Makes the Flatpak from the unpacked Linux app (`pnpm package:dir` leaves it in
// `release/linux-unpacked`): a repository holding the build, which is what Flatpak installs and
// updates from, and the same build as one file to hand someone.
//
//   node scripts/flatpak.mjs [--branch stable|beta]
//
// It needs flatpak-builder (on the `PATH`, or the `org.flatpak.Builder` Flatpak) and, installed
// from Flathub, the runtime, SDK, and base app `flatpak/com.aspen_chat.client.yml` names.
//
// `ASPEN_FLATPAK_GPG_KEY` names the GnuPG key the repository's commits and summary are signed
// with (`ASPEN_FLATPAK_GPG_HOMEDIR` its keyring, when not the default). A repository people add
// as a remote must be signed, since Flatpak takes whatever a remote's signed summary names; one
// built without a key is for looking at and installing by hand.

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { parseArgs } from "node:util";

const APP_ID = "com.aspen_chat.client";
// Where the bundle tells Flatpak to find the runtime when the machine lacks it.
const RUNTIME_REPO = "https://dl.flathub.org/repo/flathub.flatpakrepo";
const BRANCHES = ["stable", "beta"];

const root = join(import.meta.dirname, "..");
const unpacked = join(root, "release", "linux-unpacked");
const out = join(root, "release", "flatpak");
const repo = join(out, "repo");

const { values } = parseArgs({ options: { branch: { type: "string", default: "stable" } } });
const branch = values.branch;
if (!BRANCHES.includes(branch)) {
  throw new Error(`--branch is one of ${BRANCHES.join(", ")}, not ${branch}`);
}
if (!existsSync(join(unpacked, "aspen"))) {
  throw new Error(`${unpacked} holds no app; run pnpm package:dir on Linux first`);
}

function run(command, args) {
  console.log(`$ ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, { cwd: root, stdio: "inherit" });
  if (result.error !== undefined) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`${command} exited with ${result.status}`);
  }
}

function onPath(command) {
  return spawnSync(command, ["--version"], { stdio: "ignore" }).status === 0;
}

const builder = onPath("flatpak-builder")
  ? ["flatpak-builder"]
  : ["flatpak", "run", "org.flatpak.Builder"];

const key = process.env.ASPEN_FLATPAK_GPG_KEY;
const homedir = process.env.ASPEN_FLATPAK_GPG_HOMEDIR;
const signing =
  key === undefined
    ? []
    : [`--gpg-sign=${key}`, ...(homedir === undefined ? [] : [`--gpg-homedir=${homedir}`])];

const { version } = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
mkdirSync(out, { recursive: true });
writeFileSync(
  join(out, `${APP_ID}.metainfo.xml`),
  readFileSync(join(root, "flatpak", `${APP_ID}.metainfo.xml.in`), "utf8")
    .replaceAll("@VERSION@", version)
    .replaceAll("@DATE@", new Date().toISOString().slice(0, 10)),
);

// The repository is made afresh, so it holds this build and nothing left by another.
rmSync(repo, { recursive: true, force: true });
run(builder[0], [
  ...builder.slice(1),
  "--force-clean",
  `--state-dir=${join(out, "state")}`,
  `--repo=${repo}`,
  `--default-branch=${branch}`,
  ...signing,
  join(out, "build"),
  join("flatpak", `${APP_ID}.yml`),
]);
const bundle = join(out, `Aspen-${version}.flatpak`);
run("flatpak", [
  "build-bundle",
  `--runtime-repo=${RUNTIME_REPO}`,
  ...signing,
  repo,
  bundle,
  APP_ID,
  branch,
]);
console.log(`\nthe repository is ${repo}\nthe bundle is ${bundle}`);
