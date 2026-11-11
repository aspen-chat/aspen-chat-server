import type { SettingField } from "@aspen/protocol";
import { describe, expect, it } from "vitest";
import {
  changedFields,
  clearable,
  initial,
  patchOf,
  textDraft,
  type Draft,
} from "./settingsDrafts";

const key: SettingField = { name: "key", label: "key", type: "text", secret: true };
const greeting: SettingField = { name: "greeting", label: "greeting", type: "text" };
const limit: SettingField = { name: "limit", label: "limit", type: "integer" };
const fields = [key, greeting, limit];
/** A stored secret is not among the values a form is given. */
const values = { greeting: "hello", limit: 3 };

function patch(edits: Record<string, Draft>, clearing: readonly string[] = []) {
  const drafts = {
    ...Object.fromEntries(fields.map((f) => [f.name, initial(f, values)])),
    ...edits,
  };
  const cleared = new Set(clearing);
  return patchOf(changedFields(fields, drafts, values, cleared), drafts, cleared);
}

describe("plugin settings drafts", () => {
  it("sends nothing for a form left as it was", () => {
    expect(patch({})).toEqual({});
  });

  it("leaves a secret alone when its field is typed in and emptied again", () => {
    expect(patch({ key: textDraft("s") })).toEqual({ key: "s" });
    expect(patch({ key: textDraft("") })).toEqual({});
    expect(patch({ key: textDraft(""), limit: 4 })).toEqual({ limit: 4 });
  });

  it("clears a secret only when asked to", () => {
    expect(patch({}, ["key"])).toEqual({ key: null });
    expect(patch({ limit: 4 }, ["key"])).toEqual({ key: null, limit: 4 });
  });

  it("restores the default of other text that is emptied", () => {
    expect(patch({ greeting: textDraft("") })).toEqual({ greeting: null });
    expect(patch({ greeting: textDraft("hi") })).toEqual({ greeting: "hi" });
  });

  it("offers clearing for a secret that is set and not required", () => {
    expect(clearable(key, true)).toBe(true);
    expect(clearable(key, false)).toBe(false);
    expect(clearable({ ...key, required: true }, true)).toBe(false);
    expect(clearable(greeting, true)).toBe(false);
  });
});
