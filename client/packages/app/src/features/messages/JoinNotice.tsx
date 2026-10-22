import { SignInIcon } from "@phosphor-icons/react";
import { UserMention } from "@/features/messages/Mention";
import { useMessages } from "@/i18n/context";
import { formatNodes } from "@/i18n/formatNodes";

/**
 * The system message posted in a community's default channel when someone joins it, naming
 * them as a chip that opens their card.
 */
export function JoinNotice({ member }: { member: string }) {
  const m = useMessages();
  return (
    <div className="flex items-start gap-2 text-sm text-ink-muted">
      <SignInIcon size={18} aria-hidden="true" className="mt-0.5 shrink-0" />
      <span>{formatNodes(m.memberJoined, { name: <UserMention id={member} chip /> })}</span>
    </div>
  );
}
