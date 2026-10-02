/** Free helpers over channel records, which need nothing from the store. */

import type { Category, Channel } from "./generated/events";

/** Whether a channel is a DM or group DM. */
export function isDm(channel: Channel): boolean {
  return channel.ty === "dm" || channel.ty === "groupDm";
}

/** Splits a community's channels into those under each category and the top-level rest. */
export function groupChannels(
  channels: readonly Channel[],
  categories: readonly Category[],
): { topLevel: Channel[]; byCategory: Map<string, Channel[]> } {
  const byCategory = new Map<string, Channel[]>(categories.map((c) => [c.id, []]));
  const topLevel: Channel[] = [];
  for (const channel of channels) {
    const group =
      channel.parentCategory == null ? undefined : byCategory.get(channel.parentCategory);
    if (group === undefined) {
      topLevel.push(channel);
    } else {
      group.push(channel);
    }
  }
  return { topLevel, byCategory };
}
