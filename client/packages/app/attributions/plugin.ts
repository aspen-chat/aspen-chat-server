import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync, realpathSync, statSync } from "node:fs";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";
import type {
  AttributedComponent,
  AttributedPackage,
  Attributions,
  BuildInfo,
  ComponentId,
  LicenseFile,
} from "../src/features/about/attributionTypes";
import { NATIVE_LIBRARIES, PACKAGE_TEXTS, UNSHIPPED_WRAPS, type NativeLibrary } from "./native";

/**
 * Two modules made when the app is built, which the About section and the attributions page
 * read:
 *
 * - `virtual:attributions` (`Attributions`): every package each part of Aspen ships, read from
 *   where the build finds them, so the list is never older than the build. The app's npm
 *   packages are walked from `@aspen/app`'s dependencies and the phone apps' from
 *   `@aspen/mobile`'s, through `node_modules` as Node resolves them; the desktop app's are
 *   Electron, Chromium, and the capture helper's crates; the servers' are the crates the API
 *   server, the voice server, and `aspen-migrate` are built from, by `cargo metadata` for the
 *   platforms each is built for, following normal dependencies (build scripts and dev
 *   dependencies ship nothing). Each package's license files are read from its directory,
 *   with those of the C sources some crates vendor (`libwebp-sys/vendor/COPYING`); a package
 *   that carries none is given its project's (`native.ts`) or its license's standard text
 *   (`spdx/`). The libraries no package manager records are listed in `native.ts`.
 * - `virtual:build-info` (`BuildInfo`): the app's version and the commit it was built from.
 *
 * A build stops when it cannot list everything (no `cargo`, a license without a text, a
 * library in `native.ts` whose version moved), since it would otherwise ship a list that leaves
 * someone out. A development server lists what it can and names the rest in `incomplete`,
 * since it may run where the Rust toolchain is not (Playwright's Docker image).
 */
export function attributions(): Plugin {
  let strict = true;
  let made: Attributions | undefined;
  return {
    name: "aspen-attributions",
    configResolved(config) {
      strict = config.command === "build";
    },
    resolveId(id) {
      return id === ATTRIBUTIONS || id === BUILD_INFO ? `\0${id}` : undefined;
    },
    load(id) {
      if (id === `\0${ATTRIBUTIONS}`) {
        made ??= collect(strict);
        // Parsing a string is quicker than evaluating a literal this large.
        return `export default JSON.parse(${JSON.stringify(JSON.stringify(made))});`;
      }
      if (id === `\0${BUILD_INFO}`) {
        return `export default ${JSON.stringify(buildInfo())};`;
      }
      return undefined;
    },
  };
}

const ATTRIBUTIONS = "virtual:attributions";
const BUILD_INFO = "virtual:build-info";

const here = dirname(fileURLToPath(import.meta.url));
const app = dirname(here);
const client = dirname(dirname(app));
const checkout = dirname(client);

function buildInfo(): BuildInfo {
  const version = (
    JSON.parse(readFileSync(join(app, "package.json"), "utf8")) as { version: string }
  ).version;
  const repository = (
    JSON.parse(readFileSync(join(client, "package.json"), "utf8")) as { repository: string }
  ).repository;
  let commit: string | null = null;
  let modified = false;
  try {
    const git = (...args: string[]) =>
      execFileSync("git", ["-C", checkout, ...args], { encoding: "utf8" }).trim();
    commit = git("rev-parse", "--short=12", "HEAD");
    modified = git("status", "--porcelain", "--untracked-files=no") !== "";
  } catch {
    // Not a git checkout: the version alone says what was built.
  }
  return {
    version,
    commit,
    modified,
    // A build with changes of its own has no published source to point at but the project's.
    source: commit === null || modified ? repository : `${repository}/tree/${commit}`,
  };
}

/** A package found by walking a package manager's records, before its texts are read. */
interface Found {
  name: string;
  version: string | null;
  license: string;
  url: string | null;
  /** Its directory, whose license files are its texts; `null` for those `native.ts` gives. */
  dir: string | null;
  /** How deep below it license files are looked for. */
  depth: number;
  /** The license file its manifest names, relative to `dir`, should the search miss it. */
  declared?: string;
  /** The release it ships in, when its version is that release's. */
  bundledIn?: string;
  /** Files of `texts/` that `native.ts` gives it. */
  given?: readonly string[];
  /** Standard texts `native.ts` gives it beside its own. */
  spdx?: readonly string[];
  chromium?: true;
}

function collect(strict: boolean): Attributions {
  const incomplete: string[] = [];
  const attempt = (what: string, list: () => Found[]): Found[] => {
    if (strict) {
      return list();
    }
    try {
      return list();
    } catch (error) {
      console.warn(`attributions: ${what}: ${String(error)}`);
      incomplete.push(what);
      return [];
    }
  };
  const found: Record<ComponentId, Found[]> = {
    app: [
      ...attempt("npm packages of the app", () => npmPackages(app)),
      ...attempt("Noto Color Emoji", () => [notoColorEmoji()]),
    ],
    desktop: [
      ...attempt("Electron", () => electron()),
      ...attempt("crates of the capture helper", () =>
        cargoPackages(
          join(client, "packages", "desktop", "native", "obs-capture", "Cargo.toml"),
          ["aspen_obs_capture"],
          DESKTOP_PLATFORMS,
        ),
      ),
      ...attempt("libraries shipped from OBS Studio", () => obsLibraries()),
    ],
    mobile: attempt("npm packages of the phone apps", () =>
      npmPackages(join(client, "packages", "mobile")),
    ),
    server: [
      ...attempt("crates of the servers", () =>
        cargoPackages(
          join(checkout, "Cargo.toml"),
          ["aspen-chat-server", "voice_server", "aspen-migrate"],
          SERVER_PLATFORMS,
        ),
      ),
      ...attempt("libraries mediasoup builds", () => mediasoupLibraries()),
    ],
  };
  const texts = new TextTable();
  const components: AttributedComponent[] = COMPONENTS.map((id) => ({
    id,
    packages: unique(found[id])
      .map((p) => attribute(p, texts))
      .sort(
        (a, b) =>
          a.name.localeCompare(b.name, "en") || (a.version ?? "").localeCompare(b.version ?? ""),
      ),
  }));
  return { components, texts: texts.list, incomplete };
}

const COMPONENTS: readonly ComponentId[] = ["app", "desktop", "mobile", "server"];

/** The platforms the servers are built for (`scripts/cross_aarch64.py` makes the ARM ones). */
const SERVER_PLATFORMS = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"];

const DESKTOP_PLATFORMS = [
  "x86_64-pc-windows-msvc",
  "x86_64-unknown-linux-gnu",
  "aarch64-unknown-linux-gnu",
  "x86_64-apple-darwin",
  "aarch64-apple-darwin",
];

function unique(packages: Found[]): Found[] {
  const seen = new Map<string, Found>();
  for (const p of packages) {
    seen.set(`${p.name}@${p.version ?? ""}`, p);
  }
  return [...seen.values()];
}

/** Each distinct text once, by index. */
class TextTable {
  readonly list: string[] = [];
  readonly #index = new Map<string, number>();

  add(body: string): number {
    const text = body
      .replace(/\r\n?/g, "\n")
      .replace(/[ \t]+$/gm, "")
      .replace(/^\n+|\s+$/g, "");
    let index = this.#index.get(text);
    if (index === undefined) {
      index = this.list.length;
      this.list.push(text);
      this.#index.set(text, index);
    }
    return index;
  }
}

function attribute(p: Found, texts: TextTable): AttributedPackage {
  const read = (path: string) => readFileSync(path, "utf8");
  let files: LicenseFile[] = [];
  if (p.dir !== null) {
    const dir = p.dir;
    const found = licenseFiles(dir, p.depth);
    const declared = p.declared?.split(sep).join("/");
    if (declared !== undefined && !found.includes(declared)) {
      found.push(declared);
    }
    files = found.map((file) => ({ file, text: texts.add(read(join(dir, file))) }));
  }
  const given = [...(p.given ?? []), ...(PACKAGE_TEXTS[p.name] ?? [])];
  files.push(...given.map((file) => ({ file, text: texts.add(read(join(here, "texts", file))) })));
  const standard = (ids: readonly string[]) =>
    ids.map((id) => {
      const path = join(here, "spdx", `${id}.txt`);
      if (!existsSync(path)) {
        throw new Error(
          `${p.name} ${p.version ?? ""}: no license file, and no standard text for ${id} in attributions/spdx/`,
        );
      }
      return { file: id, text: texts.add(read(path)) };
    });
  files.push(...standard(p.spdx ?? []));
  const standardText = files.length === 0 && p.chromium === undefined;
  if (standardText) {
    files = standard(licenseIds(p.license));
    if (files.length === 0) {
      throw new Error(`${p.name} ${p.version ?? ""}: no license file, and no license named`);
    }
  }
  return {
    name: p.name,
    version: p.version,
    license: p.license,
    url: p.url,
    texts: files,
    standardText,
    ...(p.bundledIn === undefined ? {} : { bundledIn: p.bundledIn }),
    ...(p.chromium === undefined ? {} : { chromium: true }),
  };
}

/** The licenses and exceptions an SPDX expression names (`MIT OR Apache-2.0`, `MIT/Apache-2.0`). */
export function licenseIds(expression: string): string[] {
  return [
    ...new Set(
      expression
        .split(/\s+(?:OR|AND|WITH)\s+|[()/]/i)
        .map((id) => id.trim())
        .filter((id) => id !== ""),
    ),
  ];
}

const LICENSE_FILE = /^(licen[cs]es?|copying|copyright|notice|unlicense|patents)([-._].*)?$/i;
/** Files named like licenses that are source code, documentation pages, or data. */
const NOT_A_TEXT = /\.(rs|c|h|cc|cpp|hpp|py|js|mjs|cjs|ts|json|toml|ya?ml|html?|xml|in|sh|d\.ts)$/i;
/** Directories whose license files ship in nothing: tests, examples, and the like. */
const NOT_SHIPPED =
  /^(\..*|tests?|testdata|test-data|fuzz|fuzzer|examples?|benches|benchmarks?|docs?|node_modules|target)$/i;
/** A directory of license texts, as REUSE lays them out. */
const LICENSES_DIR = /^licen[cs]es$/i;

/**
 * The license files in `dir` and in its directories down to `depth`, where crates keep the C
 * sources they vendor, as paths relative to it, in a stable order.
 */
export function licenseFiles(dir: string, depth: number): string[] {
  const out: string[] = [];
  const visit = (path: string, level: number, all: boolean) => {
    for (const entry of readdirSync(path, { withFileTypes: true }).sort((a, b) =>
      a.name.localeCompare(b.name, "en"),
    )) {
      const full = join(path, entry.name);
      const isDir = entry.isDirectory() || (entry.isSymbolicLink() && statSync(full).isDirectory());
      if (isDir) {
        if (LICENSES_DIR.test(entry.name)) {
          visit(full, level + 1, true);
        } else if (level < depth && !NOT_SHIPPED.test(entry.name)) {
          visit(full, level + 1, false);
        }
      } else if ((all || LICENSE_FILE.test(entry.name)) && !NOT_A_TEXT.test(entry.name)) {
        out.push(relative(dir, full).split(sep).join("/"));
      }
    }
  };
  visit(dir, 0, false);
  return out;
}

interface PackageJson {
  name: string;
  version: string;
  license?: string | { type: string };
  licenses?: { type: string }[];
  homepage?: string;
  repository?: string | { url: string };
  dependencies?: Record<string, string>;
  optionalDependencies?: Record<string, string>;
}

function readPackageJson(dir: string): PackageJson {
  return JSON.parse(readFileSync(join(dir, "package.json"), "utf8")) as PackageJson;
}

/** Where `name` resolves from `from`, as Node finds it: the nearest `node_modules` above. */
function resolvePackage(name: string, from: string): string | null {
  for (let dir = from; ; dir = dirname(dir)) {
    const candidate = join(dir, "node_modules", name);
    if (existsSync(join(candidate, "package.json"))) {
      return realpathSync(candidate);
    }
    if (dirname(dir) === dir) {
      return null;
    }
  }
}

/**
 * The npm packages `root` depends on, and theirs, in production. Workspace packages (the
 * protocol package) are Aspen's own, so are walked through rather than listed.
 */
function npmPackages(root: string): Found[] {
  const out: Found[] = [];
  const seen = new Set<string>();
  const visit = (dir: string) => {
    const manifest = readPackageJson(dir);
    const optional = manifest.optionalDependencies ?? {};
    for (const name of Object.keys({ ...manifest.dependencies, ...optional })) {
      const found = resolvePackage(name, dir);
      if (found === null) {
        // An optional dependency for another platform is not installed here.
        if (name in optional) {
          continue;
        }
        throw new Error(`${name}, which ${manifest.name} depends on, is not installed`);
      }
      if (seen.has(found)) {
        continue;
      }
      seen.add(found);
      if (found.split(sep).includes("node_modules")) {
        out.push(npmPackage(found));
      }
      visit(found);
    }
  };
  visit(root);
  return out;
}

function npmPackage(dir: string): Found {
  const manifest = readPackageJson(dir);
  const license =
    typeof manifest.license === "string"
      ? manifest.license
      : (manifest.license?.type ?? manifest.licenses?.map((l) => l.type).join(" OR ") ?? "");
  const repository =
    typeof manifest.repository === "string" ? manifest.repository : manifest.repository?.url;
  return {
    name: manifest.name,
    version: manifest.version,
    license,
    url: manifest.homepage ?? (repository === undefined ? null : repositoryUrl(repository)),
    dir,
    depth: 1,
  };
}

/** A web address for an npm `repository`: `git+https://….git`, or `github:owner/name`. */
function repositoryUrl(repository: string): string {
  const shorthand = /^(?:github:)?([\w.-]+\/[\w.-]+)$/.exec(repository);
  if (shorthand !== null) {
    return `https://github.com/${shorthand[1] ?? ""}`;
  }
  return repository
    .replace(/^git\+/, "")
    .replace(/^git:\/\//, "https://")
    .replace(/\.git$/, "");
}

/**
 * Electron, which the desktop app is (a development dependency, since electron-builder copies
 * it in rather than npm installing it), and Chromium inside it, whose notices are too many to
 * carry here and ship beside the app.
 */
function electron(): Found[] {
  const dir = resolvePackage("electron", join(client, "packages", "desktop"));
  if (dir === null) {
    throw new Error("electron is not installed");
  }
  const found = npmPackage(dir);
  return [
    found,
    {
      name: "Chromium",
      version: null,
      license: "BSD-3-Clause AND others",
      url: "https://www.chromium.org",
      dir: null,
      depth: 0,
      chromium: true,
    },
  ];
}

/** The emoji font `scripts/noto_emoji.py` builds from Google's repository. */
function notoColorEmoji(): Found {
  const dir = join(app, "fonts", "noto-color-emoji");
  const manifest = JSON.parse(readFileSync(join(dir, "manifest.json"), "utf8")) as {
    source: { repository: string; commit: string };
  };
  return {
    name: "Noto Color Emoji",
    version: manifest.source.commit.slice(0, 12),
    license: "OFL-1.1",
    url: `https://github.com/${manifest.source.repository}`,
    dir,
    depth: 0,
  };
}

interface CargoMetadata {
  packages: {
    id: string;
    name: string;
    version: string;
    license: string | null;
    license_file: string | null;
    repository: string | null;
    homepage: string | null;
    manifest_path: string;
  }[];
  workspace_members: string[];
  resolve: {
    nodes: { id: string; deps: { pkg: string; dep_kinds: { kind: string | null }[] }[] }[];
  };
}

function cargoMetadata(manifest: string, platforms: readonly string[]): CargoMetadata {
  const args = ["metadata", "--format-version", "1", "--locked", "--manifest-path", manifest];
  for (const platform of platforms) {
    args.push("--filter-platform", platform);
  }
  return JSON.parse(
    execFileSync("cargo", args, { encoding: "utf8", maxBuffer: 256 * 1024 * 1024 }),
  ) as CargoMetadata;
}

/** The crates `roots` are built from on `platforms`, but the workspace's own. */
function cargoPackages(
  manifest: string,
  roots: readonly string[],
  platforms: readonly string[],
): Found[] {
  const metadata = cargoMetadata(manifest, platforms);
  const byId = new Map(metadata.packages.map((p) => [p.id, p]));
  const nodes = new Map(metadata.resolve.nodes.map((n) => [n.id, n]));
  const workspace = new Set(metadata.workspace_members);
  const pending = metadata.workspace_members.filter((id) =>
    roots.includes(byId.get(id)?.name ?? ""),
  );
  if (pending.length !== roots.length) {
    throw new Error(`${manifest} lacks one of ${roots.join(", ")}`);
  }
  const reached = new Set<string>();
  for (let id = pending.pop(); id !== undefined; id = pending.pop()) {
    if (reached.has(id)) {
      continue;
    }
    reached.add(id);
    for (const dep of nodes.get(id)?.deps ?? []) {
      if (dep.dep_kinds.some((k) => k.kind === null)) {
        pending.push(dep.pkg);
      }
    }
  }
  return [...reached]
    .filter((id) => !workspace.has(id))
    .map((id) => {
      const p = byId.get(id);
      if (p === undefined) {
        throw new Error(`cargo metadata names ${id} but does not describe it`);
      }
      const dir = dirname(p.manifest_path);
      return {
        name: p.name,
        version: p.version,
        license: p.license ?? "",
        url: p.repository ?? p.homepage,
        dir,
        depth: 3,
        ...(p.license_file === null ? {} : { declared: p.license_file }),
      };
    });
}

function native(library: NativeLibrary, version: string | null, bundledIn?: string): Found {
  return {
    name: library.name,
    version,
    ...(bundledIn === undefined ? {} : { bundledIn }),
    license: library.license,
    url: library.url,
    dir: null,
    depth: 0,
    given: library.texts,
    ...(library.spdx === undefined ? {} : { spdx: library.spdx }),
  };
}

/**
 * The libraries mediasoup's worker builds from its Meson wraps, checked against the wraps of the
 * `mediasoup-sys` the servers lock: every wrap a shipped build compiles is in `native.ts`, at the
 * version it names.
 */
function mediasoupLibraries(): Found[] {
  const metadata = cargoMetadata(join(checkout, "Cargo.toml"), SERVER_PLATFORMS);
  const sys = metadata.packages.filter((p) => p.name === "mediasoup-sys");
  if (sys.length !== 1 || sys[0] === undefined) {
    throw new Error(`expected one mediasoup-sys in Cargo.lock, found ${String(sys.length)}`);
  }
  const subprojects = join(dirname(sys[0].manifest_path), "subprojects");
  const listed = NATIVE_LIBRARIES.flatMap((library) =>
    "wrap" in library.version ? [{ library, version: library.version }] : [],
  );
  const wraps = readdirSync(subprojects)
    .filter((file) => file.endsWith(".wrap"))
    .map((file) => file.slice(0, -".wrap".length))
    .filter((wrap) => !UNSHIPPED_WRAPS.includes(wrap));
  const out: Found[] = [];
  for (const wrap of wraps) {
    const entry = listed.find((l) => l.version.wrap === wrap);
    const directory = /^directory\s*=\s*(.+)$/m
      .exec(readFileSync(join(subprojects, `${wrap}.wrap`), "utf8"))?.[1]
      ?.trim();
    if (entry === undefined) {
      throw new Error(
        `mediasoup-sys ${sys[0].version} builds the ${wrap} wrap, which attributions/native.ts does not list`,
      );
    }
    if (directory !== entry.version.directory) {
      throw new Error(
        `mediasoup-sys ${sys[0].version} builds ${String(directory)}, but attributions/native.ts lists ${entry.version.directory}; update its entry and its texts`,
      );
    }
    out.push(native(entry.library, entry.version.version));
  }
  for (const { version } of listed) {
    if (!wraps.includes(version.wrap)) {
      throw new Error(
        `attributions/native.ts lists the ${version.wrap} wrap, which mediasoup-sys ${sys[0].version} no longer has`,
      );
    }
  }
  return out;
}

/** What the Windows desktop app ships from OBS Studio's release, at the version it fetches. */
function obsLibraries(): Found[] {
  const script = readFileSync(
    join(client, "packages", "desktop", "scripts", "fetch-libobs.mjs"),
    "utf8",
  );
  const fetched = /^const OBS_VERSION = "([^"]+)";$/m.exec(script)?.[1];
  return NATIVE_LIBRARIES.flatMap((library) => {
    if (!("obs" in library.version)) {
      return [];
    }
    if (library.version.obs !== fetched) {
      throw new Error(
        `fetch-libobs.mjs fetches OBS Studio ${String(fetched)}, but attributions/native.ts lists ${library.name} from ${library.version.obs}; check what that release ships and update it`,
      );
    }
    return [
      library.version.bundled
        ? native(library, null, `OBS Studio ${library.version.obs}`)
        : native(library, library.version.obs),
    ];
  });
}
