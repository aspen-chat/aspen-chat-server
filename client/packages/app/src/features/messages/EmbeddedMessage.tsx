import type { Message } from "@aspen/protocol";
import { ArrowSquareOutIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { useLinkedMessage, useMessage, useSync, useUser } from "@/api/hooks";
import { useNameColor } from "@/features/users/nameColor";
import { Avatar } from "@/features/communities/Avatar";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { MessageBody } from "@/features/messages/MessageBody";
import { messageLink, useDomain } from "@/features/messages/links";
import { BotBadge, SystemBadge } from "@/features/users/BotBadge";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/** The frame every message shown inside another is drawn in. */
const embedClass =
  "mt-1 flex max-w-lg flex-col gap-1 rounded-md border border-line border-s-4 border-s-accent/40 bg-surface-raised p-2 text-sm";

/**
 * Another message shown for reference inside one: who wrote it and when, its text, pictures,
 * files, and link cards, and nothing to act on. `community` places it for the jump to it
 * (`null` for a DM's); without it, or for a deleted message (`deletedAt`), there is no jump, and
 * a deleted one is marked so.
 */
export function EmbeddedMessage({
  message,
  community,
  deletedAt = null,
}: {
  message: Message;
  community: string | null | undefined;
  deletedAt?: string | null;
}) {
  const m = useMessages();
  const domain = useDomain();
  const author = useUser(message.author);
  const timeFormat = useDateFormat(TIME);
  const name = author === undefined ? m.unknownUser : displayNameOf(author);
  const nameColor = useNameColor(message.author, community);
  const time = (
    <time dateTime={message.timestamp}>{timeFormat.format(new Date(message.timestamp))}</time>
  );
  return (
    <div className={embedClass}>
      <div className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
        <Avatar name={name} iconId={author?.icon ?? null} size="sm" />
        <span className="font-medium" style={{ color: nameColor }}>
          {name}
        </span>
        {author?.bot === true && <BotBadge />}
        {author?.system === true && <SystemBadge />}
        {deletedAt === null && community !== undefined ? (
          <Link
            {...messageLink({ domain, community }, message.channelId, message.id)}
            aria-label={m.reports.jumpToMessage}
            className="flex items-center gap-1 text-xs text-ink-faint outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            {time}
            <ArrowSquareOutIcon size={12} aria-hidden="true" className="rtl:-scale-x-100" />
          </Link>
        ) : (
          <>
            <span className="text-xs text-ink-faint">{time}</span>
            {deletedAt !== null && (
              <span className="rounded border border-danger/40 px-1 text-xs text-danger">
                {m.reports.deletedTag}
              </span>
            )}
          </>
        )}
      </div>
      <MessageBody message={message} home={{ domain, community: community ?? null }} still />
    </div>
  );
}

/** A line where a linked message would be, saying why it is not. */
export function EmbedNotice({ text }: { text: string }) {
  return <p className={embedClass + " text-ink-faint italic"}>{text}</p>;
}

/** Stands in for a linked message while what is there is read. */
export function EmbedSkeleton() {
  const m = useMessages();
  return (
    <div aria-busy="true" className={embedClass}>
      <LoadingLabel text={m.reports.embedLoading} />
      <Skeleton className="h-3.5 w-1/3" />
      <Skeleton className="h-3.5 w-3/4" />
    </div>
  );
}

/**
 * The messages a message links to, each beneath it as `EmbeddedMessage` draws it, as far as
 * the reader may see them: one deleted, or one they may not read, says so instead. What is
 * there comes with the history the message was read in, and for a message that arrived or was
 * edited since, from a read of its links.
 */
export function LinkedMessages({ message }: { message: Message }) {
  if (message.linkedMessages.length === 0) {
    return null;
  }
  return (
    <>
      {message.linkedMessages.map((id) => (
        <LinkedMessage key={id} id={id} from={message.id} />
      ))}
    </>
  );
}

function LinkedMessage({ id, from }: { id: string; from: string }) {
  const m = useMessages();
  const sync = useSync();
  const link = useLinkedMessage(id);
  const linked = useMessage(id);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (link === undefined && !failed) {
      sync.loadLinks(from).catch(() => {
        setFailed(true);
      });
    }
  }, [sync, link, from, failed]);
  if (link === undefined) {
    return failed ? <EmbedNotice text={m.reports.embedUnavailable} /> : <EmbedSkeleton />;
  }
  if (link.state === "unavailable") {
    return <EmbedNotice text={m.reports.embedUnavailable} />;
  }
  if (link.state === "deleted" || linked === undefined) {
    return <EmbedNotice text={m.reports.embedDeleted} />;
  }
  return <EmbeddedMessage message={linked} community={link.community ?? null} />;
}
