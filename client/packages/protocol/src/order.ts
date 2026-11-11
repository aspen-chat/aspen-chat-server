/**
 * Orders two ids, which are UUIDs in canonical lowercase form, by their code units. For UUIDv7
 * ids (messages, saves, annotations) that is the order they were made in, on every deployment.
 * Never `localeCompare`: collation is the device's language's, and some sort `aa`, `dd`, or
 * `ff`, which hex digits spell, as one letter after the others.
 */
export function compareIds(a: string, b: string): number {
  return a < b ? -1 : a > b ? 1 : 0;
}

/**
 * Orders two RFC 3339 timestamps by the instants they name. Their text does not sort that way:
 * the server writes as many fraction digits as an instant needs, so two in one second differ in
 * length.
 */
export function compareInstants(a: string, b: string): number {
  return Date.parse(a) - Date.parse(b);
}
