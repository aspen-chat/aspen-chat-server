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

/** Whether `held` is exactly the permissions of `wanted`, in any order. */
function sameSet(held: readonly Permission[], wanted: readonly Permission[]): boolean {
  const set = new Set(held);
  return set.size === wanted.length && wanted.every((permission) => set.has(permission));
}

/**
 * Which of the simple settings the overrides amount to, and the roles it names. A preset is
 * current only when the overrides are exactly `presetOverrides` of it for some roles, as sets
 * and leaving out overrides that allow and deny nothing: everyone's role denied what the preset
 * withholds and nothing else, and every other override allowing exactly that and denying
 * nothing. No overrides at all is open to everyone. Anything else is custom, since applying a
 * preset over it would take away whatever the preset does not hold.
 */
export function currentPreset(
  overrides: readonly OverrideGrant[],
  everyone: string | undefined,
): { preset: Preset; roles: readonly string[] } {
  const held = overrides.filter((o) => o.allow.length > 0 || o.deny.length > 0);
  if (held.length === 0) {
    return { preset: "everyone", roles: [] };
  }
  const base = held.find((o) => o.role === everyone);
  const others = held.filter((o) => o.role !== everyone);
  if (base === undefined) {
    return { preset: "custom", roles: [] };
  }
  for (const preset of ["private", "readOnly"] as const) {
    const withheld = withheldBy(preset);
    if (
      base.allow.length === 0 &&
      sameSet(base.deny, withheld) &&
      others.every((o) => o.deny.length === 0 && sameSet(o.allow, withheld))
    ) {
      return { preset, roles: others.map((o) => o.role) };
    }
  }
  return { preset: "custom", roles: [] };
}

/** Whether the form's selection is the setting `current` describes, roles in any order. */
export function selects(
  preset: Preset,
  chosen: ReadonlySet<string>,
  current: { preset: Preset; roles: readonly string[] },
): boolean {
  return (
    preset === current.preset &&
    chosen.size === current.roles.length &&
    current.roles.every((role) => chosen.has(role))
  );
}
