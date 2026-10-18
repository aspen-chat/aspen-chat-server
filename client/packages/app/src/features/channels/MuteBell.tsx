import type { MuteInForce } from "@aspen/protocol";
import { BellSlashIcon } from "@phosphor-icons/react";
import { Focusable } from "react-aria-components";
import { useMuteEnd } from "@/features/channels/muteEnd";
import { Tooltip } from "@/features/layout/Tooltip";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * The bell a muted channel or DM carries in its list, whose tooltip says until when, and that
 * the mute is its category's when it is. It is
 * focusable, so the tooltip follows keyboard focus as well as the pointer, and so must not sit
 * inside the row's link or button.
 */
export function MuteBell({ mute, className }: { mute: MuteInForce; className?: string }) {
  const m = useMessages();
  const end = useMuteEnd()(mute);
  const text = mute.category === null ? end : format(m.mute.byCategory, { end });
  return (
    <Tooltip text={text}>
      <Focusable>
        <span
          role="img"
          tabIndex={0}
          aria-label={text}
          className={
            "shrink-0 rounded outline-none focus-visible:ring-2 focus-visible:ring-accent/50" +
            (className === undefined ? "" : " " + className)
          }
        >
          <BellSlashIcon size={14} aria-hidden="true" />
        </span>
      </Focusable>
    </Tooltip>
  );
}
