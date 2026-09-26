import { DotsThreeVerticalIcon } from "@phosphor-icons/react";
import { Button, Menu, MenuItem, MenuTrigger, Popover } from "react-aria-components";
import { useSync } from "@/api/hooks";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const menuClass = "min-w-40 outline-none";
const itemClass =
  "cursor-default rounded px-2 py-1 text-sm outline-none focus:bg-surface-hover data-[danger]:text-danger";

/**
 * What a moderator can do to someone else in a call: server mute or unmute them, or remove
 * them. Under the Two Insanities everyone sees it for everyone but themselves.
 */
export function ParticipantMenu({
  channelId,
  userId,
  name,
  muted,
  className,
}: {
  channelId: string;
  userId: string;
  name: string;
  muted: boolean;
  className?: string;
}) {
  const m = useMessages();
  const sync = useSync();
  return (
    <MenuTrigger>
      <Button
        aria-label={format(m.voice.participantActions, { name })}
        className={
          "rounded-md p-0.5 text-ink-faint outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 " +
          (className ?? "")
        }
      >
        <DotsThreeVerticalIcon size={16} aria-hidden="true" />
      </Button>
      <Popover className="rounded-md border border-line bg-surface-raised p-1 shadow-lg">
        <Menu
          className={menuClass}
          onAction={(key) => {
            if (key === "mute") {
              void sync.muteVoiceParticipant(channelId, userId, !muted).catch(() => undefined);
            } else if (key === "kick") {
              void sync.kickVoiceParticipant(channelId, userId).catch(() => undefined);
            }
          }}
        >
          <MenuItem id="mute" className={itemClass}>
            {muted ? m.voice.serverUnmute : m.voice.serverMute}
          </MenuItem>
          <MenuItem id="kick" className={itemClass} data-danger="true">
            {m.voice.removeFromCall}
          </MenuItem>
        </Menu>
      </Popover>
    </MenuTrigger>
  );
}
