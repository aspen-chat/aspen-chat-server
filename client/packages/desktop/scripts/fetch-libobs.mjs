// Fetches the libobs the Windows build ships (`native/libobs/`, gitignored): the OBS Project's
// own release, pinned by version and checksum, with its files kept exactly as signed, because
// the game hook it injects (`graphics-hook64.dll`, with `inject-helper64.exe` and
// `get-graphics-offsets64.exe`) is whitelisted by anti-cheat systems and tolerated by antivirus
// software by that signature, which a build of our own would not carry. From the release zip:
// `obs.dll` and the libraries it and the five modules the helper loads need (ffmpeg, x264,
// pthreads, zlib, the Direct3D and WinRT graphics modules, and the rest of `bin/64bit` short of
// the OBS application, Qt, scripting, and the debug symbols), the modules themselves
// (`obs-plugins/64bit`), and their data (`data/libobs`, `data/obs-plugins/<module>`, the hook
// files among them). Linking the helper against `obs.dll` takes an import library, written here
// from the DLL's export table (`obs.def`, then `obs.lib` by MSVC's `lib` or LLVM's
// `llvm-dlltool`, whichever is found), and headers, which the source archive of the same tag
// provides (`include/`, with `obsconfig.h` written from its template). The helper's build
// script finds all of it without being told.
//
//   node scripts/fetch-libobs.mjs            # into native/libobs
//
// The hook and the `win-capture` module share a versioned interface, so everything comes from
// the one release.

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { spawnSync } from "node:child_process";
import { inflateRawSync } from "node:zlib";

const OBS_VERSION = "32.2.2";
const RELEASE_URL = `https://github.com/obsproject/obs-studio/releases/download/${OBS_VERSION}/OBS-Studio-${OBS_VERSION}-Windows-x64.zip`;
const RELEASE_SHA256 = "4d6e40e3ab155f56b30de517380566a206d74b63cdf5ad49aa596924768f97e1";
const SOURCE_URL = `https://github.com/obsproject/obs-studio/archive/refs/tags/${OBS_VERSION}.zip`;
/** The modules the helper loads (`MODULES` in its `obs.rs`) that Windows has. */
const MODULES = ["obs-x264", "obs-ffmpeg", "image-source", "win-wasapi", "win-capture"];
/**
 * What in `bin/64bit` is not a library the helper's process needs: the OBS application and its
 * tools, Qt and its plugin folders, the frontend and scripting libraries, the OpenGL graphics
 * module (Direct3D is used), and the debug symbols.
 */
const NOT_NEEDED =
  /^(obs64|obs-.*-test|obs-ffmpeg-mux)\.exe$|^Qt6|\/|^obs-frontend-api|^obs-scripting|^lua51|^libobs-opengl|\.pdb$/;

const desktop = dirname(dirname(new URL(import.meta.url).pathname.replace(/^\/([A-Za-z]:)/, "$1")));
const out = join(desktop, "native", "libobs");
const cache = join(desktop, "native", "libobs-cache");

async function download(url, name, sha256) {
  mkdirSync(cache, { recursive: true });
  const path = join(cache, name);
  if (!existsSync(path)) {
    console.log(`downloading ${url}`);
    const response = await fetch(url, { redirect: "follow" });
    if (!response.ok) {
      throw new Error(`${url}: ${response.status} ${response.statusText}`);
    }
    writeFileSync(path, Buffer.from(await response.arrayBuffer()));
  }
  const data = readFileSync(path);
  if (sha256 !== null) {
    const digest = createHash("sha256").update(data).digest("hex");
    if (digest !== sha256) {
      rmSync(path);
      throw new Error(
        `${name} is not the file pinned here (SHA-256 ${digest}); it has been removed`,
      );
    }
  }
  return data;
}

/** The entries of a zip archive, by name, each able to give its bytes. */
function zipEntries(data) {
  // The end of central directory record, found from the end past any comment.
  let eocd = -1;
  for (let i = data.length - 22; i >= Math.max(0, data.length - 65557); i--) {
    if (data.readUInt32LE(i) === 0x06054b50) {
      eocd = i;
      break;
    }
  }
  if (eocd < 0) {
    throw new Error("not a zip archive");
  }
  const count = data.readUInt16LE(eocd + 10);
  let offset = data.readUInt32LE(eocd + 16);
  const entries = new Map();
  for (let i = 0; i < count; i++) {
    if (data.readUInt32LE(offset) !== 0x02014b50) {
      throw new Error("corrupt central directory");
    }
    const method = data.readUInt16LE(offset + 10);
    const compressed = data.readUInt32LE(offset + 20);
    const nameLength = data.readUInt16LE(offset + 28);
    const extraLength = data.readUInt16LE(offset + 30);
    const commentLength = data.readUInt16LE(offset + 32);
    const local = data.readUInt32LE(offset + 42);
    const name = data.toString("utf8", offset + 46, offset + 46 + nameLength);
    entries.set(name, () => {
      const n = data.readUInt16LE(local + 26);
      const e = data.readUInt16LE(local + 28);
      const start = local + 30 + n + e;
      const raw = data.subarray(start, start + compressed);
      if (method === 0) {
        return Buffer.from(raw);
      }
      if (method === 8) {
        return inflateRawSync(raw);
      }
      throw new Error(`${name}: compression method ${method} is not handled`);
    });
    offset += 46 + nameLength + extraLength + commentLength;
  }
  return entries;
}

function extract(entries, keep, rename) {
  let files = 0;
  for (const [name, read] of entries) {
    if (name.endsWith("/") || !keep(name)) {
      continue;
    }
    const target = join(out, rename(name));
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, read());
    files += 1;
  }
  return files;
}

/** The names `obs.dll` exports, from its export table, as a module definition file. */
function exportsOf(dll) {
  const pe = dll.readUInt32LE(0x3c);
  const sections = dll.readUInt16LE(pe + 6);
  const optionalSize = dll.readUInt16LE(pe + 20);
  const magic = dll.readUInt16LE(pe + 24);
  const directories = pe + 24 + (magic === 0x20b ? 112 : 96);
  const exportRva = dll.readUInt32LE(directories);
  const table = pe + 24 + optionalSize;
  const toOffset = (rva) => {
    for (let i = 0; i < sections; i++) {
      const section = table + i * 40;
      const virtual = dll.readUInt32LE(section + 12);
      const size = Math.max(dll.readUInt32LE(section + 8), dll.readUInt32LE(section + 16));
      if (rva >= virtual && rva < virtual + size) {
        return rva - virtual + dll.readUInt32LE(section + 20);
      }
    }
    throw new Error(`RVA ${rva} is in no section`);
  };
  const directory = toOffset(exportRva);
  const names = dll.readUInt32LE(directory + 24);
  const nameTable = toOffset(dll.readUInt32LE(directory + 32));
  const exported = [];
  for (let i = 0; i < names; i++) {
    const at = toOffset(dll.readUInt32LE(nameTable + i * 4));
    const end = dll.indexOf(0, at);
    exported.push(dll.toString("ascii", at, end));
  }
  return `LIBRARY obs.dll\nEXPORTS\n${exported.map((name) => `    ${name}`).join("\n")}\n`;
}

/** Makes `obs.lib` from `obs.def` with whichever tool is at hand; says so when none is. */
function importLibrary() {
  const def = join(out, "obs.def");
  const lib = join(out, "obs.lib");
  const attempts = [
    ["lib", ["/nologo", `/def:${def}`, "/machine:x64", `/out:${lib}`]],
    ["llvm-dlltool", ["-m", "i386:x86-64", "-d", def, "-l", lib]],
    ["C:\\Program Files\\LLVM\\bin\\llvm-dlltool.exe", ["-m", "i386:x86-64", "-d", def, "-l", lib]],
  ];
  for (const [tool, args] of attempts) {
    const result = spawnSync(tool, args, { stdio: "pipe" });
    if (result.status === 0 && existsSync(lib)) {
      console.log(`wrote obs.lib with ${tool}`);
      return true;
    }
  }
  console.log(
    "no `lib` (MSVC) or `llvm-dlltool` (LLVM) found; obs.lib was not written, so the helper cannot link until one is on the PATH and this runs again",
  );
  return false;
}

async function main() {
  rmSync(out, { recursive: true, force: true });
  mkdirSync(out, { recursive: true });

  const release = zipEntries(
    await download(RELEASE_URL, `OBS-Studio-${OBS_VERSION}-Windows-x64.zip`, RELEASE_SHA256),
  );
  const modules = new Set(MODULES);
  const kept = extract(
    release,
    (name) =>
      (name.startsWith("bin/64bit/") && !NOT_NEEDED.test(name.slice("bin/64bit/".length))) ||
      MODULES.some((module) => name === `obs-plugins/64bit/${module}.dll`) ||
      name.startsWith("data/libobs/") ||
      (name.startsWith("data/obs-plugins/") &&
        modules.has(name.split("/")[2]) &&
        !name.endsWith(".pdb")),
    (name) => name,
  );
  console.log(`kept ${kept} files of OBS Studio ${OBS_VERSION}`);
  writeFileSync(join(out, "obs.def"), exportsOf(readFileSync(join(out, "bin/64bit/obs.dll"))));

  // The headers: the libobs directory of the same tag's source, with the config header its
  // build would have written, naming this directory's plugins and data as the defaults.
  const source = zipEntries(
    await download(SOURCE_URL, `obs-studio-${OBS_VERSION}-source.zip`, null),
  );
  const prefix = `obs-studio-${OBS_VERSION}/libobs/`;
  const headers = extract(
    source,
    (name) => name.startsWith(prefix) && /\.(h|hpp|inc)$/.test(name),
    (name) => join("include", name.slice(prefix.length)),
  );
  const escape = (path) => path.replace(/\\/g, "\\\\");
  writeFileSync(
    join(out, "include", "obsconfig.h"),
    [
      "#pragma once",
      `#define OBS_DATA_PATH "${escape(join(out, "data"))}"`,
      `#define OBS_PLUGIN_PATH "${escape(join(out, "obs-plugins", "64bit"))}"`,
      `#define OBS_PLUGIN_DESTINATION "${escape(join(out, "obs-plugins", "64bit"))}"`,
      "#define OBS_RELEASE_CANDIDATE 0",
      "#define OBS_BETA 0",
      "",
    ].join("\n"),
  );
  console.log(`kept ${headers} headers`);
  writeFileSync(join(out, "VERSION"), `${OBS_VERSION}\n`);
  importLibrary();
  console.log(`libobs ${OBS_VERSION} is in ${out}`);
}

main().catch((error) => {
  console.error(error.message);
  process.exit(1);
});
