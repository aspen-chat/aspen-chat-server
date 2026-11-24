/**
 * The downloads bucket (`downloads.json`) as both release scripts use it (`publishWindows.ts`,
 * `publishFlatpak.ts`): storage through the AWS CLI, the source archives every release mirrors
 * (`sources.json`), and `releases.json`.
 *
 * Storage is reached against the bucket's S3 endpoint, `ASPEN_DOWNLOADS_ENDPOINT` (Cloudflare
 * R2's `https://<account>.r2.cloudflarestorage.com`), with the CLI's own credentials
 * (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`). `--prefix` publishes beneath a folder of the
 * bucket instead of at its root, for trying an update without touching the real one;
 * `ASPEN_DOWNLOADS_URL` and `ASPEN_DOWNLOADS_BUCKET` replace the public address and the bucket,
 * for a stand-in.
 */

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, extname, join } from "node:path";
import { EMPTY_RELEASES, parseReleases, type Releases } from "./releases.ts";

export const desktop = join(import.meta.dirname, "..", "..");

/** What never changes once written: an installer, a source archive. */
export const IMMUTABLE = "public, max-age=31536000, immutable";
/** The feeds and `releases.json`, which a release replaces; a copy this old only waits longer. */
export const REPLACED = "public, max-age=300";

const CONTENT_TYPES: Readonly<Record<string, string>> = {
  ".flatpak": "application/vnd.flatpak",
  ".flatpakref": "application/vnd.flatpak.ref",
  ".flatpakrepo": "application/vnd.flatpak.repo",
  ".exe": "application/vnd.microsoft.portable-executable",
  ".gz": "application/gzip",
  ".xz": "application/x-xz",
  ".zip": "application/zip",
  ".yml": "text/yaml; charset=utf-8",
  ".json": "application/json",
};

export function contentType(file: string): string {
  const type = CONTENT_TYPES[extname(file)];
  if (type === undefined) {
    throw new Error(`no content type is known for ${file}`);
  }
  return type;
}

/**
 * One archive of `sources.json`: downloaded from where its project publishes it and checked
 * against the SHA-256 pinned for it, or, where the only archive offered is made afresh on each
 * request and so differs every time (gitiles'), made here from the commit pinned for it, which
 * git checks the fetched objects against.
 */
export type Archive =
  | { readonly file: string; readonly url: string; readonly sha256: string }
  | { readonly file: string; readonly git: string; readonly commit: string };

export function readJson(path: string): unknown {
  return JSON.parse(readFileSync(path, "utf8"));
}

export function stringField(raw: unknown, field: string, where: string): string {
  const value =
    typeof raw === "object" && raw !== null ? (raw as Record<string, unknown>)[field] : undefined;
  if (typeof value !== "string") {
    throw new Error(`${where} has no ${field}`);
  }
  return value;
}

/** Every archive of `sources.json`, of each group (`obs`, `electron`). */
export function archives(): Archive[] {
  const raw = readJson(join(desktop, "sources.json"));
  if (typeof raw !== "object" || raw === null) {
    throw new Error("sources.json is not an object");
  }
  return Object.entries(raw).flatMap(([group, value]) => {
    const list =
      typeof value === "object" && value !== null
        ? (value as Record<string, unknown>).archives
        : null;
    if (!Array.isArray(list)) {
      throw new Error(`sources.json: ${group} has no archives`);
    }
    const where = `an archive of sources.json's ${group}`;
    return list.map((entry: unknown): Archive => {
      const file = stringField(entry, "file", where);
      return typeof entry === "object" && entry !== null && "git" in entry
        ? { file, git: stringField(entry, "git", where), commit: stringField(entry, "commit", where) }
        : { file, url: stringField(entry, "url", where), sha256: stringField(entry, "sha256", where) };
    });
  });
}

/** The bucket, through the AWS CLI. */
export class Bucket {
  readonly #bucket: string;
  readonly #endpoint: string;
  readonly #prefix: string;

  constructor(bucket: string, endpoint: string, prefix: string) {
    this.#bucket = bucket;
    this.#endpoint = endpoint;
    this.#prefix = prefix;
  }

  #aws(args: string[]): string {
    return this.#cli(["s3api", ...args, "--bucket", this.#bucket, "--output", "json"]);
  }

  #cli(args: string[]): string {
    const result = spawnSync(
      "aws",
      [...args, "--endpoint-url", this.#endpoint],
      {
        encoding: "utf8",
        maxBuffer: 16 * 1024 * 1024,
        env: {
          ...process.env,
          AWS_DEFAULT_REGION: process.env.AWS_DEFAULT_REGION ?? "auto",
          // R2 takes the checksums an S3 request must carry, not every kind newer CLIs add.
          AWS_REQUEST_CHECKSUM_CALCULATION: "when_required",
          AWS_RESPONSE_CHECKSUM_VALIDATION: "when_required",
        },
      },
    );
    if (result.error !== undefined) {
      throw result.error;
    }
    if (result.status !== 0) {
      throw new Error(
        `aws ${args.slice(0, 2).join(" ")} failed (${String(result.status)}): ${result.stderr}`,
      );
    }
    return result.stdout;
  }

  key(path: string): string {
    return `${this.#prefix}${path}`;
  }

  #folder(path: string): string {
    return `s3://${this.#bucket}/${this.key(path)}/`;
  }

  /** Copies every file of the folder at `path` into `directory`. */
  download(path: string, directory: string): void {
    console.log(`reading ${this.key(path)}/`);
    this.#cli(["s3", "sync", this.#folder(path), directory, "--no-progress", "--only-show-errors"]);
  }

  /**
   * Writes the files of `directory` that `include` matches (the CLI's patterns, relative to it)
   * into the folder at `path`. `kind` says what they are: `"named"` files are named by their
   * content, so one already there is left alone; `"replaced"` files are written whether or not
   * one of their size is there, since a file of the same size may hold something else.
   */
  upload(
    directory: string,
    path: string,
    include: readonly string[],
    kind: "named" | "replaced",
    cacheControl: string,
  ): void {
    console.log(`writing ${include.join(", ")} under ${this.key(path)}/`);
    const filters = ["--exclude", "*", ...include.flatMap((pattern) => ["--include", pattern])];
    this.#cli([
      "s3",
      ...(kind === "named" ? ["sync", "--size-only"] : ["cp", "--recursive"]),
      directory,
      this.#folder(path),
      ...filters,
      "--cache-control",
      cacheControl,
      "--no-progress",
      "--only-show-errors",
    ]);
  }

  /**
   * Deletes what the folder at `path` holds and `directory` does not, but for what `exclude`
   * matches. Anything `directory` holds must be written first, since what differs is written too.
   */
  removeAbsent(directory: string, path: string, exclude: readonly string[]): void {
    console.log(`removing what ${this.key(path)}/ no longer holds`);
    this.#cli([
      "s3",
      "sync",
      "--delete",
      "--size-only",
      directory,
      this.#folder(path),
      ...exclude.flatMap((pattern) => ["--exclude", pattern]),
      "--no-progress",
    ]);
  }

  /**
   * The size of the object at `path`, or `null` when there is none. Found by listing its exact
   * key, since the CLI tells a missing object from any other failure only in its wording.
   */
  size(path: string): number | null {
    const key = this.key(path);
    const listed: unknown = JSON.parse(
      this.#aws(["list-objects-v2", "--prefix", key, "--max-keys", "1000"]) || "{}",
    );
    const contents =
      typeof listed === "object" && listed !== null
        ? (listed as Record<string, unknown>).Contents
        : undefined;
    for (const item of Array.isArray(contents) ? contents : []) {
      if (typeof item === "object" && item !== null) {
        const { Key, Size } = item as Record<string, unknown>;
        if (Key === key && typeof Size === "number") {
          return Size;
        }
      }
    }
    return null;
  }

  /** The object at `path`, or `null` when there is none. */
  read(path: string): Buffer | null {
    if (this.size(path) === null) {
      return null;
    }
    const scratch = mkdtempSync(join(tmpdir(), "aspen-publish-"));
    try {
      const file = join(scratch, "object");
      this.#aws(["get-object", "--key", this.key(path), file]);
      return readFileSync(file);
    } finally {
      rmSync(scratch, { recursive: true, force: true });
    }
  }

  /** The metadata value `name` an object was written with (`write`'s `metadata`), of one that exists. */
  metadata(path: string, name: string): string | null {
    const head: unknown = JSON.parse(this.#aws(["head-object", "--key", this.key(path)]));
    const metadata =
      typeof head === "object" && head !== null
        ? (head as Record<string, unknown>).Metadata
        : undefined;
    const value =
      typeof metadata === "object" && metadata !== null
        ? (metadata as Record<string, unknown>)[name]
        : undefined;
    return typeof value === "string" ? value : null;
  }

  write(
    path: string,
    file: string,
    cacheControl: string,
    metadata: Readonly<Record<string, string>> = {},
  ): void {
    console.log(`writing ${this.key(path)}`);
    this.#aws([
      "put-object",
      "--key",
      this.key(path),
      "--body",
      file,
      "--content-type",
      contentType(path),
      "--cache-control",
      cacheControl,
      ...(Object.keys(metadata).length === 0 ? [] : ["--metadata", JSON.stringify(metadata)]),
    ]);
  }

  writeText(path: string, text: string, cacheControl: string): void {
    const scratch = mkdtempSync(join(tmpdir(), "aspen-publish-"));
    try {
      const file = join(scratch, basename(path));
      writeFileSync(file, text);
      this.write(path, file, cacheControl);
    } finally {
      rmSync(scratch, { recursive: true, force: true });
    }
  }
}

export function digest(algorithm: "sha256" | "sha512", data: Buffer, encoding: "hex" | "base64"): string {
  return createHash(algorithm).update(data).digest(encoding);
}

/** Runs git in `directory`, failing on any failure. */
function git(directory: string, args: string[]): string {
  const result = spawnSync("git", args, { cwd: directory, encoding: "utf8" });
  if (result.error !== undefined) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`git ${args.join(" ")} failed (${String(result.status)}): ${result.stderr}`);
  }
  return result.stdout.trim();
}

/** Writes `archive` into `scratch`, as its file, checked against what `sources.json` pins. */
async function fetchArchive(archive: Archive, scratch: string): Promise<string> {
  const file = join(scratch, archive.file);
  if ("git" in archive) {
    const suffix = ".tar.gz";
    if (!archive.file.endsWith(suffix)) {
      throw new Error(`${archive.file}: an archive made from git is a .tar.gz`);
    }
    console.log(`fetching ${archive.git} at ${archive.commit}`);
    const repository = join(scratch, "repository");
    git(scratch, ["init", "--quiet", repository]);
    git(repository, ["fetch", "--quiet", "--depth", "1", archive.git, archive.commit]);
    const fetched = git(repository, ["rev-parse", "FETCH_HEAD"]);
    if (fetched !== archive.commit) {
      throw new Error(`${archive.git} gave ${fetched} for ${archive.commit}`);
    }
    git(repository, [
      "archive",
      "--format=tar.gz",
      `--prefix=${archive.file.slice(0, -suffix.length)}/`,
      "--output",
      file,
      archive.commit,
    ]);
    return file;
  }
  console.log(`fetching ${archive.url}`);
  const response = await fetch(archive.url, { redirect: "follow" });
  if (!response.ok) {
    throw new Error(`${archive.url}: ${String(response.status)} ${response.statusText}`);
  }
  const data = Buffer.from(await response.arrayBuffer());
  const found = digest("sha256", data, "hex");
  if (found !== archive.sha256) {
    throw new Error(`${archive.url} has SHA-256 ${found}, not ${archive.sha256}`);
  }
  writeFileSync(file, data);
  return file;
}

/** Copies each source archive into `sources/`, once. */
export async function mirrorSources(bucket: Bucket): Promise<void> {
  for (const archive of archives()) {
    const path = `sources/${archive.file}`;
    if (bucket.size(path) !== null) {
      continue;
    }
    const scratch = mkdtempSync(join(tmpdir(), "aspen-publish-"));
    try {
      bucket.write(path, await fetchArchive(archive, scratch), IMMUTABLE);
    } finally {
      rmSync(scratch, { recursive: true, force: true });
    }
  }
}

export function currentReleases(bucket: Bucket): Releases {
  const data = bucket.read("releases.json");
  return data === null ? EMPTY_RELEASES : parseReleases(JSON.parse(data.toString("utf8")));
}


/**
 * The bucket, and the public address of a folder in it, from the environment and `--prefix`.
 */
export function openBucket(prefix: string): {
  bucket: Bucket;
  publicUrl: (folder: string) => URL;
} {
  if (prefix !== "" && !/^[a-z0-9-]+(\/[a-z0-9-]+)*\/$/.test(prefix)) {
    throw new Error("--prefix is a folder, as `test/`");
  }
  const endpoint = process.env.ASPEN_DOWNLOADS_ENDPOINT;
  if (endpoint === undefined || endpoint === "") {
    throw new Error("ASPEN_DOWNLOADS_ENDPOINT names the bucket's S3 endpoint");
  }
  // The bucket is named on its own (`downloads.json`); an endpoint holding it too, as the address
  // Cloudflare's dashboard shows for a bucket does, would name it twice in every request.
  if (!URL.canParse(endpoint) || new URL(endpoint).pathname !== "/") {
    throw new Error(
      `ASPEN_DOWNLOADS_ENDPOINT is ${endpoint}, but it must be the account's S3 endpoint alone (https://<account>.r2.cloudflarestorage.com), without the bucket or any other path`,
    );
  }
  const downloads = readJson(join(desktop, "downloads.json"));
  const bucket = new Bucket(
    process.env.ASPEN_DOWNLOADS_BUCKET ?? stringField(downloads, "bucket", "downloads.json"),
    endpoint,
    prefix,
  );
  const root = `${process.env.ASPEN_DOWNLOADS_URL ?? stringField(downloads, "url", "downloads.json")}/`;
  return { bucket, publicUrl: (folder) => new URL(`${prefix}${folder}/`, root) };
}

/** The version every client package carries (`version.ts`), which a release publishes. */
export function packageVersion(): string {
  return stringField(readJson(join(desktop, "package.json")), "version", "package.json");
}
