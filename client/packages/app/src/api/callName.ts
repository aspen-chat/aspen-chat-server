import type { AspenSync } from "@aspen/protocol";
import { dmTitle, otherRecipients } from "@/features/dms/dmName";
import type { Messages } from "@/i18n/messages";

/** The words a DM's title needs, in the reader's language as it is now. */
let words: Pick<Messages, "unknownUser"> & { dms: Pick<Messages["dms"], "label"> } = {
  unknownUser: "",
  dms: { label: "" },
};

/** Gives the system's call names the reader's language. */
export function wordCallNames(m: Messages): void {
  words = m;
}

/**
 * What the system shows a call as (CallKit's caller name, Telecom's display name): the channel's
 * name, or for a DM the other people's names as the DM list shows them; `null` when the store
 * has no record of the channel yet, which leaves the name to the platform.
 */
export function callName(sync: AspenSync | null, channelId: string): string | null {
  const channel = sync?.store.channel(channelId);
  if (sync == null || channel === undefined) {
    return null;
  }
  if (channel.community == null) {
    const others = otherRecipients(channel, sync.store.me()?.id ?? null);
    return dmTitle(
      others.map((id) => sync.store.user(id)),
      words.unknownUser,
      words.dms.label,
    );
  }
  return channel.name;
}
