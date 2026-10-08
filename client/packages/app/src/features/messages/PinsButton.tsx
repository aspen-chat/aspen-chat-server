import { PushPinIcon, PushPinSlashIcon } from "@phosphor-icons/react";
import { Button, Dialog, DialogTrigger, Popover } from "react-aria-components";
import { useChannelCan, useMessageOnDemand, usePins, useSync } from "@/api/hooks";
import { LoadingLabel } from "@/features/layout/Skeleton";
import { Tooltip } from "@/features/layout/Tooltip";
import { messageLink, type ChannelHome } from "@/features/messages/links";
import {
  ListedMessage,
  ListedMessageSkeleton,
  listedActionClass,
} from "@/features/messages/ListedMessage";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/** The channel header's pin control and the list of the channel's pins it opens, newest first. */
export function PinsButton({
  channelId,
  channelName,
  home,
}: {
  channelId: string;
  channelName: string;
  home: ChannelHome;
}) {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Tooltip text={m.pins.show}>
        <Button
          aria-label={m.pins.show}
          className="rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          <PushPinIcon size={20} aria-hidden="true" />
        </Button>
      </Tooltip>
      <Popover
        placement="bottom end"
        className="max-h-[32rem] w-[26rem] max-w-[calc(100vw-2rem)] overflow-y-auto rounded-lg border border-line bg-surface-raised p-2 shadow-lg"
      >
        <Dialog
          aria-label={format(m.pins.heading, { channel: channelName })}
          className="outline-none"
        >
          {({ close }) => (
            <>
              <h3 className="px-1 pb-2 text-sm font-semibold">
                {format(m.pins.heading, { channel: channelName })}
              </h3>
              <PinList channelId={channelId} home={home} onJump={close} />
            </>
          )}
        </Dialog>
      </Popover>
    </DialogTrigger>
  );
}

function PinList({
  channelId,
  home,
  onJump,
}: {
  channelId: string;
  home: ChannelHome;
  onJump: () => void;
}) {
  const m = useMessages();
  const pins = usePins(channelId);
  if (pins === undefined) {
    return (
      <div aria-busy="true" className="flex flex-col gap-1">
        <LoadingLabel text={m.pins.loading} />
        {[0, 1, 2].map((index) => (
          <ListedMessageSkeleton key={index} index={index} />
        ))}
      </div>
    );
  }
  if (pins.length === 0) {
    return <p className="px-1 text-sm text-ink-muted">{m.pins.none}</p>;
  }
  return (
    <ul className="flex flex-col gap-1">
      {[...pins].reverse().map((pin) => (
        <PinnedMessage
          key={pin.messageId}
          messageId={pin.messageId}
          channelId={channelId}
          home={home}
          onJump={onJump}
        />
      ))}
    </ul>
  );
}

/** One pinned message, with a way to go to it and, for those who may pin, to unpin it. */
function PinnedMessage({
  messageId,
  channelId,
  home,
  onJump,
}: {
  messageId: string;
  channelId: string;
  home: ChannelHome;
  onJump: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const { message } = useMessageOnDemand(messageId, true);
  const mayPin = useChannelCan(channelId, "pinMessages");
  return (
    <ListedMessage
      message={message}
      home={home}
      link={messageLink(home, channelId, messageId)}
      onJump={onJump}
      actions={
        mayPin && (
          <Tooltip text={m.pins.unpin}>
            <Button
              aria-label={m.pins.unpin}
              onPress={() => {
                void sync.setPinned(messageId, false).catch(() => undefined);
              }}
              className={listedActionClass}
            >
              <PushPinSlashIcon size={16} aria-hidden="true" />
            </Button>
          </Tooltip>
        )
      }
    />
  );
}
