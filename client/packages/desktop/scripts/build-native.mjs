// Builds the capture helper and places it where the main process spawns it from:
// `native/aspen-capture`. On macOS it is one universal binary, Apple Silicon's and Intel's joined
// by `lipo`, so the arm64 and x64 apps both carry the same file and a development shell on
// either finds it; rustup is asked for both targets first.
//
// It is copied beside the destination and renamed over it, because a running shell may have the
// old helper open and a running executable cannot be overwritten in place. On Linux and macOS the
// rename succeeds, the running helper keeps the file it opened, and the next one spawned is the
// new build; Windows refuses even the rename, so there the shell must be quit first.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, renameSync } from "node:fs";
import { join } from "node:path";

const crate = join(import.meta.dirname, "..", "native", "capture");
const manifest = join(crate, "Cargo.toml");
const name = process.platform === "win32" ? "aspen-capture.exe" : "aspen-capture";
const MAC_TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

function run(command, args) {
  execFileSync(command, args, { stdio: "inherit" });
}

let built;
if (process.platform === "darwin") {
  run("rustup", ["target", "add", ...MAC_TARGETS]);
  for (const target of MAC_TARGETS) {
    run("cargo", ["build", "--release", "--manifest-path", manifest, "--target", target]);
  }
  const universal = join(crate, "target", "universal");
  mkdirSync(universal, { recursive: true });
  built = join(universal, name);
  run("lipo", [
    "-create",
    "-output",
    built,
    ...MAC_TARGETS.map((target) => join(crate, "target", target, "release", name)),
  ]);
} else {
  run("cargo", ["build", "--release", "--manifest-path", manifest]);
  built = join(crate, "target", "release", name);
}

const destination = join(crate, "..", name);
const staged = `${destination}.new`;
copyFileSync(built, staged);
renameSync(staged, destination);
console.log(`copied the capture helper to native/${name}`);
