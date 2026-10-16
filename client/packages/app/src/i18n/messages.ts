import catalogue from "./locales/en.json";

/**
 * User-facing strings: `locales/en.json`, the English catalogue every other language is
 * translated from, nested by feature. Keys are camelCase to match the server's `locales/*.yml`
 * convention. Server-produced Problem `title`/`detail` values are already localized and are
 * shown as-is.
 */
export const en = catalogue;

/** A catalogue: the shape of `en`, with any text in place of its strings. */
type Catalogue<T> = { readonly [K in keyof T]: T[K] extends string ? string : Catalogue<T[K]> };

export type Messages = Catalogue<typeof en>;

export function format(template: string, values: Record<string, string>): string {
  return template.replace(/\{(\w+)\}/g, (match, key: string) => values[key] ?? match);
}
