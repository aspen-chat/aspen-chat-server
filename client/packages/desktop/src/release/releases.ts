/**
 * What a release of the desktop app publishes, decided without touching storage, so the rules are
 * tested here and `publish.ts` only carries them out: which versions may be released, what
 * `releases.json` says once one is, and the update feeds electron-updater reads
 * (`docs/architecture/desktop-updates.md`).
 */

/**
 * A version the desktop app is released as: `X.Y.Z` on the stable track, `X.Y.Z-beta.N` on the
 * beta track. Nothing else is accepted, so every version orders the same way here, in semver, and
 * in electron-updater.
 */
export interface Version {
  readonly major: number;
  readonly minor: number;
  readonly patch: number;
  /** `N` of `-beta.N`; `null` for a stable version. */
  readonly beta: number | null;
}

export type Track = "stable" | "beta";

const NUMBER = "(0|[1-9][0-9]{0,8})";
const VERSION = new RegExp(`^${NUMBER}\\.${NUMBER}\\.${NUMBER}(?:-beta\\.${NUMBER})?$`);

export function parseVersion(text: string): Version | null {
  const match = VERSION.exec(text);
  if (match === null) {
    return null;
  }
  const [, major, minor, patch, beta] = match;
  return {
    major: Number(major),
    minor: Number(minor),
    patch: Number(patch),
    beta: beta === undefined ? null : Number(beta),
  };
}

export function formatVersion(version: Version): string {
  const core = `${String(version.major)}.${String(version.minor)}.${String(version.patch)}`;
  return version.beta === null ? core : `${core}-beta.${String(version.beta)}`;
}

/** Semver's order: a beta comes before the stable version it is a candidate for. */
export function compareVersions(a: Version, b: Version): number {
  const core = a.major - b.major || a.minor - b.minor || a.patch - b.patch;
  if (core !== 0 || a.beta === b.beta) {
    return core;
  }
  if (a.beta === null) {
    return 1;
  }
  if (b.beta === null) {
    return -1;
  }
  return a.beta - b.beta;
}

export function trackOf(version: Version): Track {
  return version.beta === null ? "stable" : "beta";
}

/** The version a `client-v…` tag names, or `null` for a tag that names none. */
export function versionOfTag(tag: string): Version | null {
  const prefix = "client-v";
  return tag.startsWith(prefix) ? parseVersion(tag.slice(prefix.length)) : null;
}

/** One installer, as `releases.json` lists it. */
export interface Build {
  readonly version: string;
  /** Where it is downloaded from. */
  readonly url: string;
  /** Hexadecimal, for a person checking what they downloaded. */
  readonly sha256: string;
  /** Base64, as electron-updater checks a download. */
  readonly sha512: string;
  /** In bytes. */
  readonly size: number;
  /** An ISO 8601 instant. */
  readonly released: string;
}

export interface Channels {
  readonly stable: Build | null;
  readonly beta: Build | null;
}

/**
 * `releases.json`: the newest build on each track, by platform and architecture
 * (`windows-x64`). The beta track holds the newest of either, so it never falls behind stable.
 * The site's download page reads it, and the publish job reads it back to decide the next.
 */
export interface Releases {
  readonly builds: Readonly<Record<string, Channels>>;
}

export const EMPTY_RELEASES: Releases = { builds: {} };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseBuild(raw: unknown, where: string): Build | null {
  if (raw === null) {
    return null;
  }
  if (
    !isRecord(raw) ||
    typeof raw.version !== "string" ||
    parseVersion(raw.version) === null ||
    typeof raw.url !== "string" ||
    typeof raw.sha256 !== "string" ||
    typeof raw.sha512 !== "string" ||
    typeof raw.size !== "number" ||
    typeof raw.released !== "string"
  ) {
    throw new Error(`releases.json: ${where} is not a build`);
  }
  return {
    version: raw.version,
    url: raw.url,
    sha256: raw.sha256,
    sha512: raw.sha512,
    size: raw.size,
    released: raw.released,
  };
}

/** Reads `releases.json` as published, refusing one this code did not write. */
export function parseReleases(raw: unknown): Releases {
  if (!isRecord(raw) || !isRecord(raw.builds)) {
    throw new Error("releases.json has no builds");
  }
  const builds: Record<string, Channels> = {};
  for (const [key, channels] of Object.entries(raw.builds)) {
    if (!isRecord(channels)) {
      throw new Error(`releases.json: ${key} is not a pair of tracks`);
    }
    builds[key] = {
      stable: parseBuild(channels.stable ?? null, `${key}.stable`),
      beta: parseBuild(channels.beta ?? null, `${key}.beta`),
    };
  }
  return { builds };
}

function newer(build: Build, than: Build | null): boolean {
  if (than === null) {
    return true;
  }
  const a = parseVersion(build.version);
  const b = parseVersion(than.version);
  if (a === null || b === null) {
    throw new Error(`${build.version} or ${than.version} is not a release version`);
  }
  return compareVersions(a, b) > 0;
}

/**
 * `releases.json` once `build` is released for `key`, and which tracks it now heads. A version
 * no newer than the head of its own track is refused: its feed would offer everyone an older
 * build than they may hold, which electron-updater would never install, and a version already
 * released would be released twice. A stable build heads the beta track as well when it is newer
 * than the beta there, so beta testers move on to it.
 */
export function release(
  current: Releases,
  key: string,
  build: Build,
): { releases: Releases; tracks: Track[] } {
  const version = parseVersion(build.version);
  if (version === null) {
    throw new Error(`${build.version} is not a release version (X.Y.Z or X.Y.Z-beta.N)`);
  }
  const was = current.builds[key] ?? { stable: null, beta: null };
  const track = trackOf(version);
  if (!newer(build, was[track])) {
    throw new Error(
      `${build.version} is no newer than ${String(was[track]?.version)}, the ${track} track's ${key} build`,
    );
  }
  const tracks: Track[] = [track];
  if (track === "stable" && newer(build, was.beta)) {
    tracks.push("beta");
  }
  const channels: Channels = {
    stable: tracks.includes("stable") ? build : was.stable,
    beta: tracks.includes("beta") ? build : was.beta,
  };
  return { releases: { builds: { ...current.builds, [key]: channels } }, tracks };
}

/** The feed electron-updater reads for each track, beside the installers it names. */
export const FEED_FILES: Readonly<Record<Track, string>> = {
  stable: "latest.yml",
  beta: "beta.yml",
};

/** YAML's single-quoted scalar, which needs only its own quote doubled. */
function quoted(text: string): string {
  return `'${text.replaceAll("'", "''")}'`;
}

/**
 * A feed naming `build`, its installer at `path` (relative to the feed, as electron-updater
 * resolves it). It carries no blockmap, so updates download whole.
 */
export function feedText(build: Build, path: string): string {
  return [
    `version: ${quoted(build.version)}`,
    "files:",
    `  - url: ${quoted(path)}`,
    `    sha512: ${quoted(build.sha512)}`,
    `    size: ${String(build.size)}`,
    `path: ${quoted(path)}`,
    `sha512: ${quoted(build.sha512)}`,
    `releaseDate: ${quoted(build.released)}`,
    "",
  ].join("\n");
}

/** The Windows installer for x64, as `releases.json` keys it and where it is kept. */
export const WINDOWS_X64 = {
  key: "windows-x64",
  folder: "windows/x64",
  installer: (version: string) => `${version}/Aspen-Setup-${version}.exe`,
} as const;

/** The Flatpak for x64, as `releases.json` keys its bundle and where the bundle is kept. */
export const LINUX_X64_FLATPAK = {
  key: "linux-x64-flatpak",
  folder: "linux/x64",
  bundle: (version: string) => `${version}/Aspen-${version}.flatpak`,
} as const;
