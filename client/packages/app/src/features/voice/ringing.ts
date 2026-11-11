import type { ServerClock, VoiceRing } from "@aspen/protocol";

/**
 * Whether a ring is still in force at `deviceNow`, a reading of this device's clock. A ring
 * ends at its `until` with no event, and `until` is its server's, so it is judged on that
 * server's `clock`: on the device's own, a clock half a minute fast would never ring at all.
 */
export function ringing(ring: VoiceRing, clock: ServerClock, deviceNow: number): boolean {
  return Date.parse(ring.until) > clock.at(deviceNow);
}
