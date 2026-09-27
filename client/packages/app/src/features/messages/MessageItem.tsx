import type { LinkPreview } from "@aspen/protocol";
import { ArrowBendDownRightIcon, ChatsCircleIcon, PencilSimpleIcon } from "@phosphor-icons/react";
import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { useChannel, useMe, useMessage, useSync, useUser } from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { Tooltip } from "@/features/layout/Tooltip";
import { ProfilePopover } from "@/features/users/ProfileCard";
import { displayNameOf } from "@/features/users/profile";
import { DeleteMessageDialog } from "@/features/messages/DeleteMessageDialog";
import { MessageMedia } from "@/features/messages/Attachments";
import { imageUrls, isImageUrl, onlyImageLinks } from "@/features/messages/images";
import { Markdown } from "@/features/messages/Markdown";
import { VideoCard } from "@/features/messages/VideoCard";
import { playerSrc } from "@/features/messages/video";
import { MessageEditor } from "@/features/messages/MessageEditor";
import { PollCard } from "@/features/messages/PollCard";
import { PollClosedNotice } from "@/features/messages/PollClosedNotice";
import { ReactionChips, ReactionPicker } from "@/features/messages/Reactions";
import { messageLink, threadLink, type ChannelHome } from "@/features/messages/links";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const timeFormat = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });

const actionClass =
  "rounded px-2 py-0.5 text-xs text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * One message. `parentId` is the parent channel when `channelId` is a thread: its messages
 * link to the thread and cannot start threads of their own. Elsewhere a message offers to
 * start a thread, and one that started a thread shows its replies' summary; an echo shows the
 * thread reply it names.
 */
export function MessageItem({
  id,
  home,
  channelId,
  parentId,
  highlighted,
  threadable = true,
}: {
  id: string;
  home: ChannelHome;
  channelId: string;
  parentId: string | null;
  highlighted: boolean;
  /** Whether the message may start or show a thread here; not for a thread's own header. */
  threadable?: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const message = useMessage(id);
  const author = useUser(message?.author);
  const me = useMe();
  const [editing, setEditing] = useState(false);
  if (message === undefined) {
    return null;
  }
  const inThread = parentId !== null;
  const canThread =
    threadable && !inThread && message.kind !== "threadEcho" && message.kind !== "pollClosed";
  const permalink = inThread
    ? threadLink(home, parentId, channelId)
    : messageLink(home, channelId, id);
  const openThread = () => {
    if (message.thread != null) {
      void navigate(threadLink(home, channelId, message.thread));
      return;
    }
    sync.openThread(id).then(
      (thread) => {
        void navigate(threadLink(home, channelId, thread.id));
      },
      () => undefined,
    );
  };
  // Editing and deleting are offered only on the caller's own messages. The server accepts
  // either from anyone for now, but the controls should not invite it.
  const own = me !== null && me.id === message.author;
  // A poll message has no text of its own; its card is edited by voting, not by rewriting.
  const editable = own && message.kind === "standard";
  const linkedImages = imageUrls(message.content);
  // A link the client already shows as a picture needs no card from the server for the same URL.
  const previews = message.linkPreviews.filter((p) => !linkedImages.includes(p.url));
  // Links the server found to be images themselves join the message's pictures; the rest
  // are cards.
  const previewImages = previews.flatMap((p) =>
    isPictureOnly(p) && p.imageUrl != null ? [{ src: p.imageUrl, name: p.url }] : [],
  );
  const cards = previews.filter((p) => !isPictureOnly(p));
  // A message that is nothing but links to pictures shows the pictures alone.
  const pictureOnly = onlyImageLinks(
    message.content,
    (url) => isImageUrl(url) || previews.some((p) => p.url === url && isPictureOnly(p)),
  );
  if (message.kind === "pollClosed" && message.poll != null) {
    return (
      <article
        data-message-id={id}
        className={
          "flex gap-3 rounded-md px-2 py-1.5 " +
          (highlighted ? "bg-accent-soft" : "hover:bg-surface-hover/60")
        }
      >
        <div className="w-10 shrink-0" aria-hidden="true" />
        <div className="min-w-0 flex-1">
          <PollClosedNotice pollId={message.poll} home={home} channelId={channelId} />
        </div>
      </article>
    );
  }
  return (
    <article
      data-message-id={id}
      className={
        "group flex gap-3 rounded-md px-2 py-1.5 " +
        (highlighted ? "bg-accent-soft" : "hover:bg-surface-hover/60")
      }
    >
      <Avatar name={author === undefined ? "?" : displayNameOf(author)} iconId={author?.icon} />
      <div className="min-w-0 flex-1">
        <div className="flex items-baseline gap-2">
          {author === undefined ? (
            <span className="font-medium">{m.unknownUser}</span>
          ) : (
            <ProfilePopover user={author}>
              <Button
                aria-label={format(m.profile.show, { name: displayNameOf(author) })}
                className="rounded font-medium outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
              >
                {displayNameOf(author)}
              </Button>
            </ProfilePopover>
          )}
          {message.kind === "threadEcho" && (
            <span className="flex items-center gap-1 text-xs text-ink-muted">
              <ArrowBendDownRightIcon size={12} aria-hidden="true" />
              {m.threads.repliedInThread}
            </span>
          )}
          <Link
            {...permalink}
            className="text-xs text-ink-faint outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
            title={m.linkToMessage}
          >
            <time dateTime={message.timestamp}>
              {timeFormat.format(new Date(message.timestamp))}
            </time>
          </Link>
          {!editing && (
            <span
              role="group"
              aria-label={m.messageActionsLabel}
              className="ml-auto flex gap-1 opacity-0 group-hover:opacity-100 focus-within:opacity-100"
            >
              <ReactionPicker messageId={id} triggerClassName={actionClass} />
              {canThread && (
                <Tooltip text={m.threads.replyInThread}>
                  <Button
                    onPress={openThread}
                    aria-label={m.threads.replyInThread}
                    className={actionClass}
                  >
                    <ChatsCircleIcon size={16} aria-hidden="true" />
                  </Button>
                </Tooltip>
              )}
              {editable && (
                <Tooltip text={m.editMessage}>
                  <Button
                    onPress={() => {
                      setEditing(true);
                    }}
                    aria-label={m.editMessage}
                    className={actionClass}
                  >
                    <PencilSimpleIcon size={16} aria-hidden="true" />
                  </Button>
                </Tooltip>
              )}
              {own && (
                <DeleteMessageDialog
                  messageId={id}
                  triggerClassName={actionClass + " text-danger"}
                />
              )}
            </span>
          )}
        </div>
        {editing ? (
          <MessageEditor
            messageId={id}
            initial={message.content}
            onDone={() => {
              setEditing(false);
            }}
          />
        ) : message.kind === "threadEcho" && message.echoOf != null ? (
          <EchoedReply replyId={message.echoOf} home={home} channelId={channelId} />
        ) : (
          <div className="flex flex-wrap items-baseline gap-x-1">
            {!pictureOnly && <Markdown content={message.content} />}
            {message.editedAt != null && (
              <span
                className="text-xs text-ink-faint"
                title={format(m.editedAt, { date: timeFormat.format(new Date(message.editedAt)) })}
              >
                {m.edited}
              </span>
            )}
          </div>
        )}
        {message.kind === "poll" && message.poll != null && <PollCard pollId={message.poll} />}
        <MessageMedia
          attachmentIds={message.attachments}
          linkedImages={linkedImages}
          previewImages={previewImages}
        />
        {cards.map((preview) => (
          <LinkPreviewCard key={preview.url} preview={preview} />
        ))}
        <ReactionChips messageId={id} />
        {canThread && message.thread != null && (
          <ThreadSummary threadId={message.thread} home={home} channelId={channelId} />
        )}
      </div>
    </article>
  );
}

/**
 * The thread reply an echo shows, read from the reply itself so its edits show here too, with
 * a way into the thread. A reply the cache lacks is fetched.
 */
function EchoedReply({
  replyId,
  home,
  channelId,
}: {
  replyId: string;
  home: ChannelHome;
  channelId: string;
}) {
  const m = useMessages();
  const sync = useSync();
  const reply = useMessage(replyId);
  const [missing, setMissing] = useState(false);
  useEffect(() => {
    if (reply === undefined) {
      sync.loadMessage(replyId).catch(() => {
        setMissing(true);
      });
    }
  }, [sync, replyId, reply]);
  if (reply === undefined) {
    return missing ? (
      <p className="text-sm text-ink-faint italic">{m.threads.replyDeleted}</p>
    ) : null;
  }
  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap items-baseline gap-x-1">
        <Markdown content={reply.content} />
        {reply.editedAt != null && <span className="text-xs text-ink-faint">{m.edited}</span>}
      </div>
      <MessageMedia attachmentIds={reply.attachments} linkedImages={[]} previewImages={[]} />
      <Link
        {...threadLink(home, channelId, reply.channelId)}
        className="w-fit text-xs text-accent outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
      >
        {m.threads.viewThread}
      </Link>
    </div>
  );
}

/** Under a message that started a thread: how many replies it has and when the last came. */
function ThreadSummary({
  threadId,
  home,
  channelId,
}: {
  threadId: string;
  home: ChannelHome;
  channelId: string;
}) {
  const m = useMessages();
  const thread = useChannel(threadId);
  const count = thread?.replyCount ?? 0;
  const label =
    count === 0
      ? m.threads.viewThread
      : count === 1
        ? m.threads.oneReply
        : format(m.threads.replies, { count: String(count) });
  const last = thread?.lastReplyAt;
  return (
    <Link
      {...threadLink(home, channelId, threadId)}
      className="mt-1 flex w-fit items-center gap-1.5 rounded-md px-1 py-0.5 text-sm font-medium text-accent outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      <ChatsCircleIcon size={16} aria-hidden="true" />
      <span>{label}</span>
      {count > 0 && last != null && (
        <span className="font-normal text-ink-faint">
          · {format(m.threads.lastReply, { time: timeFormat.format(new Date(last)) })}
        </span>
      )}
    </Link>
  );
}

/** A preview the server made from a link that is itself an image: a picture, no text. */
function isPictureOnly(preview: LinkPreview): boolean {
  return preview.imageUrl != null && preview.title == null && preview.description == null;
}

function LinkPreviewCard({ preview }: { preview: LinkPreview }) {
  const player = preview.video == null ? null : playerSrc(preview.video.src);
  if (preview.video != null && player !== null) {
    return <VideoCard preview={preview} player={player} />;
  }
  const title = preview.title ?? preview.siteName ?? preview.url;
  return (
    <a
      href={preview.url}
      target="_blank"
      rel="noreferrer"
      className="mt-1 flex max-w-lg gap-3 rounded-md border border-line bg-surface-raised p-2 text-sm hover:bg-surface-hover"
      style={
        preview.themeColor != null
          ? { borderLeftColor: preview.themeColor, borderLeftWidth: 3 }
          : undefined
      }
    >
      {preview.imageUrl != null && (
        <img src={preview.imageUrl} alt="" className="h-16 w-16 shrink-0 rounded object-cover" />
      )}
      <span className="min-w-0">
        <span className="block truncate font-medium text-accent">{title}</span>
        {preview.description != null && (
          <span className="mt-0.5 line-clamp-2 block text-ink-muted">{preview.description}</span>
        )}
      </span>
    </a>
  );
}
