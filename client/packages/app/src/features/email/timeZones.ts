/** The time zone this device keeps, which a digest is first sent by. */
export function deviceTimeZone(): string {
  return Intl.DateTimeFormat().resolvedOptions().timeZone;
}

/**
 * Every time zone the browser knows, in order, with UTC, which some leave out, and `current`, so
 * a zone chosen on another device is always among them.
 */
export function timeZones(current: string): string[] {
  const known = new Set(Intl.supportedValuesOf("timeZone"));
  known.add("UTC");
  known.add(current);
  return [...known].sort();
}
