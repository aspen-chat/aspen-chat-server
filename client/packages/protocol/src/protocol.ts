/**
 * The Aspen protocol versions this client speaks, which it checks another deployment speaks too
 * before signing in there (`spec/federation.md`).
 */
export interface Protocol {
  readonly version: number;
  readonly minimum: number;
  readonly capabilities?: readonly string[];
}

/** This client's: the newest version it knows and the oldest it still speaks. */
export const CLIENT_PROTOCOL: Protocol = { version: 1, minimum: 1, capabilities: [] };

/** The version two sides speak to each other in, the newest both know; `null` when none. */
export function commonVersion(a: Protocol, b: Protocol): number | null {
  const version = Math.min(a.version, b.version);
  return version >= a.minimum && version >= b.minimum ? version : null;
}

/** Whether a deployment has a capability beyond its version's baseline. */
export function supports(protocol: Protocol, capability: string): boolean {
  return protocol.capabilities?.includes(capability) ?? false;
}
