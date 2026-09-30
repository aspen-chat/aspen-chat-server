import { PushPinIcon, PushPinSlashIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { useEffect } from "react";
import { Button, Dialog, DialogTrigger, Popover } from "react-aria-components";
import { useBlocked, useChannelCan, useMessage, usePins, useSync, useUser } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { Tooltip } from "@/features/layout/Tooltip";
import { messageLink, type ChannelHome } from "@/features/messages/links";
import { MessageBody } from "@/features/messages/MessageBody";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

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

/**
 * One pinned message: its author's picture and name, and the message drawn as the channel draws
 * it (`MessageBody`: its text, tags, pictures, files, poll, and link cards), with a way to go to it and, for those who may pin, to unpin it. A
 * message by someone the reader blocked is not quoted; going to it opens it.
 */
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
  const blocked = useBlocked(message?.author);
  const mayPin = useChannelCan(channelId, "pinMessages");
  const time = useDateFormat(TIME);
  useEffect(() => {
    if (message === undefined) {
      void sync.loadMessage(messageId).catch(() => undefined);
    }
  }, [sync, messageId, message]);
  const name = author === undefined ? m.unknownUser : displayNameOf(author);
  return (
    <li className="flex gap-2 rounded-md px-2 py-1.5 hover:bg-surface-hover">
      <Avatar name={name} iconId={author?.icon} size="sm" />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5 text-sm">
        <div className="flex items-baseline gap-2">
          <span className="truncate font-medium">{name}</span>
          {message !== undefined && (
            <span className="shrink-0 text-xs text-ink-faint">
              {time.format(new Date(message.timestamp))}
            </span>
          )}
          <span className="ms-auto flex shrink-0 items-center gap-1">
            <Link
              {...messageLink(home, channelId, messageId)}
              onClick={onJump}
              className="rounded px-1 text-xs text-accent outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
            >
              {m.pins.jump}
            </Link>
            {mayPin && (
              <Tooltip text={m.pins.unpin}>
                <Button
                  aria-label={m.pins.unpin}
                  onPress={() => {
                    void sync.setPinned(messageId, false).catch(() => undefined);
                  }}
                  className="tap-target rounded p-0.5 text-ink-muted outline-none hover:bg-surface-raised hover:text-ink pressed:bg-surface-raised focus-visible:ring-2 focus-visible:ring-accent/50"
                >
                  <PushPinSlashIcon size={16} aria-hidden="true" />
                </Button>
              </Tooltip>
            )}
          </span>
        </div>
        {blocked ? (
          <span className="text-ink-faint italic">{m.blocking.oneBlockedMessage}</span>
        ) : message === undefined ? (
          <span className="text-ink-muted">{m.loading}</span>
        ) : (
          <div className="flex min-w-0 flex-col gap-1 break-words">
            <MessageBody message={message} home={home} />
          </div>
        )}
      </div>
    </li>
  );
}
