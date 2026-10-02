import { PushPinIcon, PushPinSlashIcon } from "@phosphor-icons/react";
import { Button } from "react-aria-components";
import { usePins, useSync } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { ACTION_ICON } from "@/features/messages/actionIcon";
import { useMessages } from "@/i18n/context";

/** Pins the message in its channel, or unpins it. */
export function PinButton({
  messageId,
  channelId,
  className,
}: {
  messageId: string;
  channelId: string;
  className: string;
}) {
  const m = useMessages();
  const sync = useSync();
  const pins = usePins(channelId);
  const pinned = pins?.some((p) => p.messageId === messageId) ?? false;
  const label = pinned ? m.pins.unpin : m.pins.pin;
  return (
    <Tooltip text={label}>
      <Button
        aria-label={label}
        isDisabled={pins === undefined}
        onPress={() => {
          void sync.setPinned(messageId, !pinned).catch(() => undefined);
        }}
        className={className}
      >
        {pinned ? (
          <PushPinSlashIcon size={ACTION_ICON} aria-hidden="true" />
        ) : (
          <PushPinIcon size={ACTION_ICON} aria-hidden="true" />
        )}
      </Button>
    </Tooltip>
  );
}
