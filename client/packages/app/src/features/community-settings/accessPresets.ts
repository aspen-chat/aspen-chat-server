import type { OverrideGrant, Permission } from "@aspen/protocol";

/**
 * The plain settings a channel's access leads with: open to everyone, private to some roles,
 * or read-only except for some roles; `custom` is overrides that are none of these.
 */
export type Preset = "everyone" | "private" | "readOnly" | "custom";

/** One of the settings offered, as a radio choice shows it. */
export interface PresetOption {
  key: Preset;
  label: string;
  hint: string;
}

/** What a read-only channel withholds from everyone but the chosen roles: taking part. */
const POSTING: readonly Permission[] = [
  "sendMessages",
  "sendInThreads",
  "startThreads",
  "createPolls",
];

/** What a preset withholds from everyone's role and gives back to the chosen roles. */
export function withheldBy(preset: Preset): readonly Permission[] {
  return preset === "private" ? ["viewChannel"] : preset === "readOnly" ? POSTING : [];
}

/**
 * The overrides a preset amounts to: everyone's role denied what it withholds and each chosen
 * role allowed it back. Open to everyone is no overrides at all.
 */
export function presetOverrides(
  preset: Preset,
  chosen: ReadonlySet<string>,
  everyone: string,
): OverrideGrant[] {
  const withheld = withheldBy(preset);
  if (withheld.length === 0) {
    return [];
  }
  return [
    { role: everyone, allow: [], deny: [...withheld] },
    ...Array.from(chosen, (role) => ({ role, allow: [...withheld], deny: [] })),
  ];
}

/**
 * Which of the simple settings the overrides amount to, and the roles it names: everyone's
 * role denied viewing is private to the roles allowed it; denied sending, read-only for those
 * allowed to send; no overrides at all is open to everyone; anything else is custom.
 */
export function currentPreset(
  overrides: readonly OverrideGrant[],
  everyone: string | undefined,
): { preset: Preset; roles: readonly string[] } {
  const base = overrides.find((o) => o.role === everyone);
  const others = overrides.filter((o) => o.role !== everyone);
  if (base?.deny.includes("viewChannel") === true) {
    return {
      preset: "private",
      roles: others.filter((o) => o.allow.includes("viewChannel")).map((o) => o.role),
    };
  }
  if (base?.deny.includes("sendMessages") === true) {
    return {
      preset: "readOnly",
      roles: others.filter((o) => o.allow.includes("sendMessages")).map((o) => o.role),
    };
  }
  if (overrides.every((o) => o.allow.length === 0 && o.deny.length === 0)) {
    return { preset: "everyone", roles: [] };
  }
  return { preset: "custom", roles: [] };
}
