import { PushPinIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { useEffect } from "react";
import { Button, Dialog, DialogTrigger, Popover } from "react-aria-components";
import { useMessage, usePins, useSync, useUser } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { messageLink, type ChannelHome } from "@/features/messages/links";
import { displayNameOf } from "@/features/users/profile";
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
        className="max-h-96 w-80 overflow-y-auto rounded-lg border border-line bg-surface-raised p-2 shadow-lg"
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
    return <p className="px-1 text-sm text-ink-muted">{m.pins.loading}</p>;
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

/** One pinned message in brief: who wrote it and how it begins, linking to it in place. */
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
  const message = useMessage(messageId);
  const author = useUser(message?.author);
  useEffect(() => {
    if (message === undefined) {
      void sync.loadMessage(messageId).catch(() => undefined);
    }
  }, [sync, messageId, message]);
  return (
    <li>
      <Link
        {...messageLink(home, channelId, messageId)}
        onClick={onJump}
        aria-label={m.pins.jump}
        className="flex flex-col rounded-md px-2 py-1.5 text-sm outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
      >
        <span className="font-medium">
          {author === undefined ? m.unknownUser : displayNameOf(author)}
        </span>
        <span className="line-clamp-2 text-ink-muted">{message?.content ?? m.loading}</span>
      </Link>
    </li>
  );
}
