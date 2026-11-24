import { parseVersion, trackOf, type Track } from "../release/releases.ts";

/** What the reader chose in Settings: a track to follow, or no updates at all. */
export type UpdateChoice = Track | "off";

export function parseChoice(raw: unknown): UpdateChoice | null {
  return raw === "stable" || raw === "beta" || raw === "off" ? raw : null;
}

/**
 * The choice of someone who has made none: the track the installed build came from, so a beta
 * installed by hand goes on receiving betas. A version this build cannot read (a development
 * build's) follows stable.
 */
export function defaultChoice(installed: string): Track {
  const version = parseVersion(installed);
  return version === null ? "stable" : trackOf(version);
}

/**
 * Whether a downloaded update may install under `choice`. Following stable never installs a
 * beta, though one downloaded while following beta may still be waiting when the reader switches.
 */
export function mayInstall(choice: UpdateChoice, version: string): boolean {
  const parsed = parseVersion(version);
  switch (choice) {
    case "off":
      return false;
    case "beta":
      return parsed !== null;
    case "stable":
      return parsed !== null && trackOf(parsed) === "stable";
  }
}
