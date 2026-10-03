import type { ChannelMute } from "@aspen/protocol";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

/** When a mute ends: the date and time, which a mute of a week at most needs no year for. */
const UNTIL: Intl.DateTimeFormatOptions = {
  month: "short",
  day: "numeric",
  hour: "numeric",
  minute: "2-digit",
};

/** Until when a mute lasts, as the Mute submenu and the muted bell's tooltip both say it. */
export function useMuteEnd(): (mute: ChannelMute) => string {
  const m = useMessages();
  const until = useDateFormat(UNTIL);
  return (mute) =>
    mute.until == null
      ? m.mute.mutedForGood
      : format(m.mute.mutedUntil, { time: until.format(new Date(mute.until)) });
}
