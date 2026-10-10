// electron-builder's `afterPack` hook: fails a Windows build whose packaged `libobs/` is not,
// byte for byte, what `fetch-libobs.mjs` fetched. A file that differs has been signed again or
// otherwise rewritten on its way into the package, and no longer carries the signature the OBS
// Project gave it (see `fetch-libobs.mjs` for why that signature matters).
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";

const fetched = join(import.meta.dirname, "..", "native", "libobs");

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

/** Every file under `directory`, as paths relative to it. */
function filesUnder(directory) {
  return readdirSync(directory, { recursive: true, withFileTypes: true })
    .filter((entry) => entry.isFile())
    .map((entry) => relative(directory, join(entry.parentPath, entry.name)));
}

export default function checkLibobs(context) {
  if (context.electronPlatformName !== "win32") {
    return;
  }
  const packaged = join(context.appOutDir, "resources", "libobs");
  const files = filesUnder(packaged);
  if (!files.includes("VERSION")) {
    throw new Error(`${packaged} holds no libobs; run pnpm fetch:libobs before packaging`);
  }
  const changed = files.filter((file) => {
    const original = join(fetched, file);
    return !existsSync(original) || sha256(original) !== sha256(join(packaged, file));
  });
  if (changed.length > 0) {
    throw new Error(
      `packaging changed the OBS Project's files, which must ship as fetched: ` +
        `${changed.join(", ")}. If electron-builder signed them, add them to win.signExts in ` +
        `electron-builder.yml.`,
    );
  }
}
