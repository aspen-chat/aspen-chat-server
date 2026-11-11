import type { SettingField } from "@aspen/protocol";

/**
 * A plugin setting's value as its form holds it, before it becomes what the server takes.
 * `null` is a field with nothing in it: for a secret, which is never read back, that says
 * nothing of the value kept, so it is cleared only by asking to (`clearable`).
 */
export type Draft = string | boolean | number | readonly string[] | null;

/** The choice that is no choice. */
export const NONE = "";

/** What the form starts from for `field`: the value set, its default, or nothing. */
export function initial(field: SettingField, values: Record<string, unknown>): Draft {
  const value = values[field.name] ?? field.default ?? null;
  switch (field.type) {
    case "boolean":
      return value === true;
    case "integer":
      return typeof value === "number" ? value : null;
    case "textList":
    case "roleList":
    case "channelList":
      return Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : [];
    default:
      return typeof value === "string" ? value : null;
  }
}

/**
 * A line of text as its draft: nothing once it is empty, so a field typed in and emptied again
 * is as it was before it was touched.
 */
export function textDraft(text: string): Draft {
  return text === "" ? null : text;
}

export function isList(draft: Draft): draft is readonly string[] {
  return Array.isArray(draft);
}

/** Whether two drafts hold the same. */
function same(a: Draft, b: Draft): boolean {
  if (isList(a) && isList(b)) {
    return a.length === b.length && a.every((v, i) => v === b[i]);
  }
  return a === b;
}

/** What the server takes for a draft: `null` restores the setting's default. */
function wire(field: SettingField, draft: Draft): unknown {
  if (field.type === "textList" && isList(draft)) {
    return draft.map((line) => line.trim()).filter((line) => line !== "");
  }
  if (typeof draft === "string" && draft === "" && field.type !== "longText") {
    return null;
  }
  return draft;
}

/**
 * Whether `field` is a secret that can be cleared: one with a value kept, which the plugin
 * does not require (the server refuses to leave a required setting without one).
 */
export function clearable(field: SettingField, secretSet: boolean): boolean {
  return field.type === "text" && field.secret === true && secretSet && field.required !== true;
}

/**
 * The fields a save would change: those whose draft differs from what the form started from
 * (`values`), and the secrets in `clearing`.
 */
export function changedFields(
  fields: readonly SettingField[],
  drafts: Record<string, Draft>,
  values: Record<string, unknown>,
  clearing: ReadonlySet<string>,
): SettingField[] {
  return fields.filter(
    (f) => clearing.has(f.name) || !same(drafts[f.name] ?? null, initial(f, values)),
  );
}

/** The patch that saves `changed`: each as the server takes it, a cleared secret as `null`. */
export function patchOf(
  changed: readonly SettingField[],
  drafts: Record<string, Draft>,
  clearing: ReadonlySet<string>,
): Record<string, unknown> {
  return Object.fromEntries(
    changed.map((f) => [f.name, clearing.has(f.name) ? null : wire(f, drafts[f.name] ?? null)]),
  );
}
