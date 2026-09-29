/**
 * The languages the user reads, most preferred first, as BCP 47 tags. Every `AspenClient`
 * names them in `Accept-Language` and every event stream in `?locale=`, so each deployment
 * writes its errors in the language the app shows; the app sets them from its own language
 * setting. Until it does, requests carry whatever the platform sends.
 */
let preferred: string | null = null;

/** Sets the languages every request names from now on; an empty list names none. */
export function setPreferredLanguages(tags: readonly string[]): void {
  preferred = tags.length === 0 ? null : tags.join(", ");
}

/** The `Accept-Language` value for the languages set, or `null` when none are. */
export function acceptLanguage(): string | null {
  return preferred;
}
