/**
 * The client's version, which every package of `client/` carries in its `package.json`: the
 * desktop installer and its update feed take the desktop package's, and About Aspen shows the
 * app package's (`attributions/plugin.ts`). A release sets it in one commit and tags that commit
 * `client-v<version>`:
 *
 *   node src/release/version.ts set 0.2.0-beta.1
 *   node src/release/version.ts check client-v0.2.0-beta.1
 *
 * `check`, which the release job runs first, fails unless the tag names a release version
 * (`parseVersion`) and every package carries it.
 */

import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { formatVersion, parseVersion, versionOfTag, type Version } from "./releases.ts";

const client = join(import.meta.dirname, "..", "..", "..", "..");

function packageFiles(): string[] {
  const packages = join(client, "packages");
  return [
    join(client, "package.json"),
    ...readdirSync(packages, { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => join(packages, entry.name, "package.json")),
  ];
}

function read(file: string): Record<string, unknown> {
  const raw: unknown = JSON.parse(readFileSync(file, "utf8"));
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) {
    throw new Error(`${file} is not a package`);
  }
  return raw as Record<string, unknown>;
}

function set(version: Version): void {
  const text = formatVersion(version);
  for (const file of packageFiles()) {
    writeFileSync(file, `${JSON.stringify({ ...read(file), version: text }, null, 2)}\n`);
    console.log(`${file}: ${text}`);
  }
}

function check(version: Version): void {
  const text = formatVersion(version);
  const differing = packageFiles()
    .filter((file) => read(file).version !== text)
    .map((file) => `client/${relative(client, file)}`);
  if (differing.length > 0) {
    throw new Error(
      `the tag names ${text}, but ${differing.join(", ")} carr${differing.length === 1 ? "ies" : "y"} another version: set it with \`node src/release/version.ts set ${text}\`, commit, and tag that commit`,
    );
  }
  console.log(`every client package is ${text}`);
}

const [command, argument, ...rest] = process.argv.slice(2);
if (argument === undefined || rest.length > 0) {
  throw new Error("say `set <version>` or `check <tag>`");
}
if (command === "set") {
  const version = parseVersion(argument);
  if (version === null) {
    throw new Error(`${argument} is not a release version (X.Y.Z or X.Y.Z-beta.N)`);
  }
  set(version);
} else if (command === "check") {
  const version = versionOfTag(argument);
  if (version === null) {
    throw new Error(`${argument} is not a release tag (client-vX.Y.Z or client-vX.Y.Z-beta.N)`);
  }
  check(version);
} else {
  throw new Error("say `set <version>` or `check <tag>`");
}
