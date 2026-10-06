import { isDm } from "@aspen/protocol";
import { useChannel } from "@/api/hooks";
import { useMessages } from "@/i18n/context";

/** How a thread's parent is named where a reply may be shown in it: `#name`, or the DM. */
export function useEchoTarget(parentId: string): string {
  const m = useMessages();
  const parent = useChannel(parentId);
  return parent === undefined || isDm(parent) ? m.threads.thisConversation : `#${parent.name}`;
}
