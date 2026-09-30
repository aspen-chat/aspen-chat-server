import {
  ArrowBendDownRightIcon,
  ChatsCircleIcon,
  PencilSimpleIcon,
  PushPinIcon,
  PushPinSlashIcon,
  RobotIcon,
} from "@phosphor-icons/react";
import { Link, useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { Button } from "react-aria-components";
import {
  useChannel,
  useChannelAccess,
  useMe,
  useMessage,
  usePins,
  useSync,
  useUser,
  useUserLoading,
} from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { Tooltip } from "@/features/layout/Tooltip";
import { BotBadge } from "@/features/users/BotBadge";
import { ProfilePopover } from "@/features/users/ProfileCard";
import { displayNameOf } from "@/features/users/profile";
import { DeleteMessageDialog } from "@/features/messages/DeleteMessageDialog";
import { MessageMedia } from "@/features/messages/Attachments";
import { Markdown } from "@/features/messages/Markdown";
import { MessageBody } from "@/features/messages/MessageBody";
import { MessageEditor } from "@/features/messages/MessageEditor";
import { CallNotice, MissedCallNotice } from "@/features/messages/CallNotice";
import { PollClosedNotice } from "@/features/messages/PollClosedNotice";
import { ReactionChips, ReactionPicker, ViewReactionsButton } from "@/features/messages/Reactions";
import { messageLink, threadLink, type ChannelHome } from "@/features/messages/links";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";
import { CopyIdButton } from "@/features/layout/CopyId";
import { UserMention } from "@/features/messages/Mention";
import { formatNodes } from "@/i18n/formatNodes";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

// On a touch screen the actions sit side by side at finger size.
const actionClass =
  "rounded px-2 py-0.5 text-xs text-ink-muted outline-none hover:bg-surface-hover hover:text-ink " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50 " +
  "pointer-coarse:p-3";

/**
 * One message. `parentId` is the parent channel when `channelId` is a thread: its messages
 * link to the thread and cannot start threads of their own. Elsewhere a message offers to
 * start a thread, and one that started a thread shows its replies' summary; an echo shows the
 * thread reply it names.
 *
 * Its actions show while the pointer is over it or focus is in it. A touch screen has no
 * hover, so tapping a message focuses it and shows them, and tapping elsewhere hides them.
 * There they float over the message's corner, hidden rather than transparent until shown: a
 * tap on a link in the message focuses the message first (Safari gives links no focus), and
 * actions that moved the message's content, or caught taps while unseen, would take the tap.
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
  const timeFormat = useDateFormat(TIME);
  const m = useMessages();
  const sync = useSync();
  const navigate = useNavigate();
  const message = useMessage(id);
  const author = useUser(message?.author);
  const authorLoading = useUserLoading(message?.author);
  const me = useMe();
  const permissions = useChannelAccess(channelId);
  const [editing, setEditing] = useState(false);
  if (message === undefined) {
    return null;
  }
  const inThread = parentId !== null;
  // Opening a thread that exists is reading it; starting one takes Start threads.
  const canThread =
    threadable &&
    !inThread &&
    message.kind !== "threadEcho" &&
    message.kind !== "pollClosed" &&
    message.kind !== "call" &&
    message.kind !== "missedCall" &&
    (message.thread != null || permissions.has("startThreads"));
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
  // A message is edited only by its author, and deleted by its author or by someone who may
  // manage messages here.
  const own = me !== null && me.id === message.author;
  const deletable = own || permissions.has("manageMessages");
  // A poll message has no text of its own; its card is edited by voting, not by rewriting.
  const editable = own && message.kind === "standard";
  if (message.kind === "call" || message.kind === "missedCall") {
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
          {message.kind === "call" ? (
            <CallNotice starter={message.author} seconds={message.callSeconds ?? 0} />
          ) : (
            <MissedCallNotice caller={message.author} />
          )}
        </div>
      </article>
    );
  }
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
  // A message that tags the reader stands out, with a bar at its edge in place of padding.
  const tagsMe = sync.store.mentionsMe(message);
  return (
    <article
      data-message-id={id}
      data-mentions-me={tagsMe ? "true" : undefined}
      tabIndex={-1}
      className={
        "group relative flex gap-3 rounded-md py-1.5 outline-none " +
        (tagsMe ? "border-s-2 border-accent pe-2 ps-1.5 " : "px-2 ") +
        (highlighted
          ? "bg-accent-soft"
          : tagsMe
            ? "bg-accent-soft/50"
            : "hover:bg-surface-hover/60 focus-within:bg-surface-hover/60")
      }
    >
      {author === undefined ? (
        authorLoading ? (
          <Skeleton className="h-9 w-9 shrink-0 rounded-full" />
        ) : (
          <Avatar name="?" />
        )
      ) : (
        // The picture opens the same card as the name. It stays out of the tab order, where the
        // name already offers the card, so a keyboard does not stop on each message twice.
        <ProfilePopover user={author}>
          <Button
            aria-label={format(m.profile.show, { name: displayNameOf(author) })}
            excludeFromTabOrder
            className="h-fit shrink-0 rounded-full outline-none pressed:opacity-80 focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <Avatar name={displayNameOf(author)} iconId={author.icon} />
          </Button>
        </ProfilePopover>
      )}
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
          {author === undefined ? (
            authorLoading ? (
              <>
                <LoadingLabel />
                <Skeleton className="h-3.5 w-24 self-center" />
              </>
            ) : (
              <span className="font-medium">{m.unknownUser}</span>
            )
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
          {author?.bot === true && <BotBadge />}
          {message.kind === "command" && message.commandBot != null && (
            <span className="flex items-center gap-1 text-xs text-ink-muted">
              <RobotIcon size={12} aria-hidden="true" />
              {formatNodes(m.commands.sentTo, {
                bot: <UserMention id={message.commandBot} chip />,
              })}
            </span>
          )}
          {message.kind === "threadEcho" && (
            <span className="flex items-center gap-1 text-xs whitespace-nowrap text-ink-muted">
              <ArrowBendDownRightIcon size={12} aria-hidden="true" className="rtl:-scale-x-100" />
              {m.threads.repliedInThread}
            </span>
          )}
          <Link
            {...permalink}
            className="text-xs whitespace-nowrap text-ink-faint outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
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
              className="ms-auto flex gap-1 opacity-0 group-hover:opacity-100 group-focus-within:opacity-100 pointer-coarse:invisible pointer-coarse:absolute pointer-coarse:-top-4 pointer-coarse:end-2 pointer-coarse:z-10 pointer-coarse:rounded-lg pointer-coarse:border pointer-coarse:border-line pointer-coarse:bg-surface-raised pointer-coarse:shadow-md pointer-coarse:group-focus-within:visible"
            >
              {permissions.has("addReactions") && (
                <ReactionPicker messageId={id} triggerClassName={actionClass} />
              )}
              {permissions.has("pinMessages") && (
                <PinButton messageId={id} channelId={channelId} className={actionClass} />
              )}
              <ViewReactionsButton messageId={id} triggerClassName={actionClass} />
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
              {deletable && (
                <DeleteMessageDialog
                  messageId={id}
                  triggerClassName={actionClass + " text-danger"}
                />
              )}
              <CopyIdButton id={id} thing="message" className={actionClass} />
            </span>
          )}
        </div>
        {editing ? (
          <MessageEditor
            messageId={id}
            channelId={message.channelId}
            initial={message.content}
            onDone={() => {
              setEditing(false);
            }}
          />
        ) : message.kind === "threadEcho" && message.echoOf != null ? (
          <EchoedReply replyId={message.echoOf} home={home} channelId={channelId} />
        ) : null}
        <MessageBody
          message={message}
          home={home}
          hideText={editing || (message.kind === "threadEcho" && message.echoOf != null)}
          {...(own || permissions.has("manageMessages")
            ? {
                onRemoveAttachment: (attachmentId: string) => {
                  void sync.removeAttachment(id, attachmentId).catch(() => undefined);
                },
              }
            : {})}
        />
        <ReactionChips messageId={id} canReact={permissions.has("addReactions")} />
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
    ) : (
      <div aria-busy="true" className="flex flex-col gap-1.5 py-0.5">
        <LoadingLabel />
        <Skeleton className="h-3.5 w-3/4" />
        <Skeleton className="h-3 w-20" />
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-1">
      <div className="flex flex-wrap items-baseline gap-x-1">
        <Markdown content={reply.content} mentions={reply.mentions} communityId={home.community} />
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
  const timeFormat = useDateFormat(TIME);
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
      className="mt-1 flex w-fit flex-wrap items-center gap-x-1.5 rounded-md px-1 py-0.5 text-sm font-medium text-accent outline-none hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      <ChatsCircleIcon size={16} aria-hidden="true" />
      <span className="whitespace-nowrap">{label}</span>
      {count > 0 && last != null && (
        <span className="font-normal text-ink-faint">
          · {format(m.threads.lastReply, { time: timeFormat.format(new Date(last)) })}
        </span>
      )}
    </Link>
  );
}

/** Pins the message in its channel, or unpins it. */
function PinButton({
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
          <PushPinSlashIcon size={16} aria-hidden="true" />
        ) : (
          <PushPinIcon size={16} aria-hidden="true" />
        )}
      </Button>
    </Tooltip>
  );
}
