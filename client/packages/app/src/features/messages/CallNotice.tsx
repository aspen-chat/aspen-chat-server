import { PhoneIcon, XIcon } from "@phosphor-icons/react";
import { useLocale } from "react-aria-components";
import { useUser } from "@/api/hooks";
import { callLength } from "@/features/messages/callLength";
import { UserMention } from "@/features/messages/Mention";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { formatNodes } from "@/i18n/formatNodes";
import { format } from "@/i18n/messages";

/** The record a DM's call leaves when it ends: who started it, and how long it lasted. */
export function CallNotice({ starter, seconds }: { starter: string; seconds: number }) {
  const m = useMessages();
  const { locale } = useLocale();
  const user = useUser(starter);
  return (
    <div className="flex items-start gap-2 text-sm text-ink-muted">
      <PhoneIcon size={18} aria-hidden="true" className="mt-0.5 shrink-0" />
      <span>
        {format(m.voice.callRecord, {
          name: user === undefined ? m.unknownUser : displayNameOf(user),
          length: callLength(locale, seconds),
        })}
      </span>
    </div>
  );
}

/**
 * The record a DM's call leaves when no one joined whoever started it: a missed call, with a
 * red X, naming the caller as a chip that opens their card.
 */
export function MissedCallNotice({ caller }: { caller: string }) {
  const m = useMessages();
  return (
    <div className="flex items-start gap-2 text-sm text-ink-muted">
      <XIcon size={18} weight="bold" aria-hidden="true" className="mt-0.5 shrink-0 text-danger" />
      <span>{formatNodes(m.voice.missedCall, { name: <UserMention id={caller} chip /> })}</span>
    </div>
  );
}
