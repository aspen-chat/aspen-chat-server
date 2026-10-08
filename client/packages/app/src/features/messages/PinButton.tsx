import { PushPinIcon, PushPinSlashIcon } from "@phosphor-icons/react";
import { usePins, useSync } from "@/api/hooks";
import { IconAction } from "@/features/layout/IconAction";
import { ACTION_ICON } from "@/features/messages/actionIcon";
import { useMessages } from "@/i18n/context";

/** Pins the message in its channel, or unpins it. */
export function PinButton({
  messageId,
  channelId,
  className,
  labelled = false,
  onPressed,
}: {
  messageId: string;
  channelId: string;
  className: string;
  /** Drawn with its name beside its icon, as a row of a list. */
  labelled?: boolean;
  /** Called as it is pressed, for a sheet offering it to close. */
  onPressed?: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const pins = usePins(channelId);
  const pinned = pins?.some((p) => p.messageId === messageId) ?? false;
  const label = pinned ? m.pins.unpin : m.pins.pin;
  return (
    <IconAction
      label={label}
      labelled={labelled}
      isDisabled={pins === undefined}
      onPress={() => {
        void sync.setPinned(messageId, !pinned).catch(() => undefined);
        onPressed?.();
      }}
      className={className}
      icon={
        pinned ? (
          <PushPinSlashIcon size={ACTION_ICON} aria-hidden="true" />
        ) : (
          <PushPinIcon size={ACTION_ICON} aria-hidden="true" />
        )
      }
    />
  );
}
