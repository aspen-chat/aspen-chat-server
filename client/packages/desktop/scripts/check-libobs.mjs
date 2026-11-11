// electron-builder's `afterPack` hook: fails a Windows build whose packaged OBS files are not,
// byte for byte, what `fetch-libobs.mjs` fetched: `libobs/`, and the GPU encoders' test programs
// beside the helper (`obs-*-test.exe`), each of which must be there, or the encoder whose module
// runs it silently never appears. A file that differs has been signed again or otherwise
// rewritten on its way into the package, and no longer carries the signature the OBS Project
// gave it (see `fetch-libobs.mjs` for why that signature matters). It also fails a build that
// lacks a module the helper loads on Windows (`MODULES` in its `obs/startup.rs`, read from the
// source): the helper skips a module that is not there, and its sources or encoder are then
// simply absent.
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";

const native = join(import.meta.dirname, "..", "native");
const fetched = join(native, "libobs");
const startup = join(native, "capture", "src", "obs", "startup.rs");

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

/** Every file under `directory`, as paths relative to it. */
function filesUnder(directory) {
  return readdirSync(directory, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => relative(directory, join(entry.parentPath, entry.name)));
}

/**
 * The modules the helper loads on Windows: the names in the `MODULES` list of its
 * `obs/startup.rs`. Throws when the list is not found as it is declared there, rather than
 * checking nothing.
 */
export function helperModules(source) {
  const list = /^const MODULES: &\[&str\] = &\[([^\]]*)\];\r?$/m.exec(source);
  const names = list === null ? [] : [...list[1].matchAll(/"([^"]+)"/g)].map(([, name]) => name);
  if (names.length === 0) {
    throw new Error(`${startup} no longer declares MODULES the way check-libobs.mjs reads it`);
  }
  return names;
}

export default function checkLibobs(context) {
  if (context.electronPlatformName !== "win32") {
    return;
  }
  const resources = join(context.appOutDir, "resources");
  const packaged = join(resources, "libobs");
  const files = filesUnder(packaged);
  if (!files.includes("VERSION")) {
    throw new Error(`${packaged} holds no libobs; run pnpm fetch:libobs before packaging`);
  }
  const unpackaged = helperModules(readFileSync(startup, "utf8")).filter(
    (name) => !files.includes(join("obs-plugins", "64bit", `${name}.dll`)),
  );
  if (unpackaged.length > 0) {
    throw new Error(
      `the helper loads OBS modules the package lacks: ${unpackaged.join(", ")}. ` +
        `Add them to MODULES in fetch-libobs.mjs, run pnpm fetch:libobs, and package again.`,
    );
  }
  // Each packaged file against the one fetched, by its path under `libobs/`, and each test
  // program fetched against the one packaged beside the helper.
  const pairs = files.map((file) => [join(fetched, file), join(packaged, file), `libobs/${file}`]);
  const programs = readdirSync(native).filter((file) => /^obs-.*-test\.exe$/.test(file));
  if (programs.length === 0) {
    throw new Error(`${native} holds no encoder test programs; run pnpm fetch:libobs again`);
  }
  for (const program of programs) {
    pairs.push([join(native, program), join(resources, program), program]);
  }
  const missing = pairs.filter(([original, copy]) => !existsSync(original) || !existsSync(copy));
  if (missing.length > 0) {
    throw new Error(
      `packaged OBS files and the fetched ones do not match up: ` +
        `${missing.map(([, , label]) => label).join(", ")}. Run pnpm fetch:libobs and package again.`,
    );
  }
  const changed = pairs.filter(([original, copy]) => sha256(original) !== sha256(copy));
  if (changed.length > 0) {
    throw new Error(
      `packaging changed the OBS Project's files, which must ship as fetched: ` +
        `${changed.map(([, , label]) => label).join(", ")}. If electron-builder signed them, ` +
        `add them to win.signExts in electron-builder.yml.`,
    );
  }
}
