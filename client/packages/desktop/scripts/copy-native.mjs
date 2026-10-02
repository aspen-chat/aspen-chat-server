// Places the capture helper where the main process spawns it from: `native/aspen-obs-capture`.
// It is copied beside the destination and renamed over it, because a running shell may have the
// old helper open and a running executable cannot be overwritten in place. On Linux and macOS the
// rename succeeds, the running helper keeps the file it opened, and the next one spawned is the
// new build; Windows refuses even the rename, so there the shell must be quit first.
import { copyFileSync, existsSync, renameSync } from "node:fs";
import { join } from "node:path";

const crate = join(import.meta.dirname, "..", "native", "obs-capture");
const name = process.platform === "win32" ? "aspen-obs-capture.exe" : "aspen-obs-capture";
const source = join(crate, "target", "release", name);
if (!existsSync(source)) {
  throw new Error(`${source} is missing; run cargo build --release in ${crate}`);
}
const destination = join(crate, "..", name);
const staged = `${destination}.new`;
copyFileSync(source, staged);
renameSync(staged, destination);
console.log(`copied the capture helper to native/${name}`);
