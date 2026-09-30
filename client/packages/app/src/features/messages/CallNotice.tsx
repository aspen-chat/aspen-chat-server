import { PhoneIcon } from "@phosphor-icons/react";
import { useLocale } from "react-aria-components";
import { useUser } from "@/api/hooks";
import { callLength } from "@/features/messages/callLength";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
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
