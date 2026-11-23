/**
 * The files that point Flatpak at Aspen's repository: `aspen.flatpakrepo`, which adds the
 * repository as a remote, and a `.flatpakref` per track, which software centres and
 * `flatpak install` open to install the app from it in one step, adding Flathub for the runtime
 * where the machine lacks it. Both carry the repository's public key, which Flatpak then checks
 * every commit and summary against.
 */

import type { Track } from "./releases.ts";

export const APP_ID = "com.aspen_chat.client";
/** Where the runtime comes from, for a machine that has no remote holding it. */
export const RUNTIME_REPO = "https://dl.flathub.org/repo/flathub.flatpakrepo";
/** The remote's name on people's machines. */
export const REMOTE_NAME = "aspen";

export interface Repository {
  /** The repository's address, ending in `/`. */
  readonly url: string;
  readonly homepage: string;
  /** The public key, as the binary OpenPGP packets, which the files carry in base64. */
  readonly key: Buffer;
}

/** The files' own format, a desktop-entry-style key file. */
function keyFile(group: string, entries: readonly [string, string][]): string {
  for (const [key, value] of entries) {
    if (/[\n\r]/.test(value)) {
      throw new Error(`${key} holds a line break`);
    }
  }
  return `[${group}]\n${entries.map(([key, value]) => `${key}=${value}`).join("\n")}\n`;
}

export function flatpakrepo(repository: Repository): string {
  return keyFile("Flatpak Repo", [
    ["Title", "Aspen"],
    ["Url", repository.url],
    ["Homepage", repository.homepage],
    ["Comment", "Aspen, the community chat app"],
    ["GPGKey", repository.key.toString("base64")],
  ]);
}

export function flatpakref(repository: Repository, track: Track): string {
  return keyFile("Flatpak Ref", [
    ["Name", APP_ID],
    ["Branch", track],
    ["Title", track === "stable" ? "Aspen" : "Aspen (beta)"],
    ["Url", repository.url],
    ["SuggestRemoteName", REMOTE_NAME],
    ["Homepage", repository.homepage],
    ["IsRuntime", "false"],
    ["RuntimeRepo", RUNTIME_REPO],
    ["GPGKey", repository.key.toString("base64")],
  ]);
}

/** The `.flatpakref` file of each track, beside `aspen.flatpakrepo`. */
export const FLATPAKREF_FILES: Readonly<Record<Track, string>> = {
  stable: "aspen.flatpakref",
  beta: "aspen-beta.flatpakref",
};
