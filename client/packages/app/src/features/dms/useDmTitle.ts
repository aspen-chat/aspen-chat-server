import type { Channel } from "@aspen/protocol";
import { useMe, useUsers } from "@/api/hooks";
import { dmTitle, otherRecipients } from "@/features/dms/dmName";
import { useMessages } from "@/i18n/context";

/** What to call a DM: the other people's names, kept current as their profiles change. */
export function useDmTitle(channel: Channel): string {
  const m = useMessages();
  const me = useMe();
  const users = useUsers(otherRecipients(channel, me?.id ?? null));
  return dmTitle(users, m.unknownUser, m.dms.label);
}
