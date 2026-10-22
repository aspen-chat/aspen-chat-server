import type { Channel, Community } from "@aspen/protocol";

/**
 * The channel a community opens at: its default channel when the reader may view it, which is
 * where new members' join messages are posted, and otherwise its first text channel.
 */
export function landingChannel(
  community: Community | undefined,
  channels: readonly Channel[],
): Channel | undefined {
  const preferred = community?.defaultChannel;
  return (
    (preferred == null ? undefined : channels.find((c) => c.id === preferred)) ??
    channels.find((c) => c.ty === "text")
  );
}
