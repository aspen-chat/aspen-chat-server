import { ArrowBendDownRightIcon, ChatsCircleIcon, RobotIcon } from "@phosphor-icons/react";
import { Link, useNavigate } from "@tanstack/react-router";
import type { Message, User } from "@aspen/protocol";
import { memo, useEffect, useRef, useState, type ReactNode } from "react";
// React Aria Components has no long press; the hook it is built on does.
import { useLongPress } from "react-aria";
import { Button } from "react-aria-components";
import {
  useChannel,
  useChannelAccess,
  useMe,
  useMessage,
  useSync,
  useUser,
  useUserLoading,
} from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { BotBadge, SystemBadge } from "@/features/users/BotBadge";
import { ProfilePopover } from "@/features/users/ProfileCard";
import { displayNameOf } from "@/features/users/profile";
import { useNameColor } from "@/features/users/nameColor";
import { MessageMedia } from "@/features/messages/Attachments";
import { Markdown } from "@/features/messages/Markdown";
import { MessageBody } from "@/features/messages/MessageBody";
import { MessageAnnotations } from "@/features/plugins/Annotations";
import { MessageEditor } from "@/features/messages/MessageEditor";
import { CallNotice, MissedCallNotice } from "@/features/messages/CallNotice";
import { PollClosedNotice } from "@/features/messages/PollClosedNotice";
import {
  ReactionChips,
  ReactionPickerOverlay,
  ReactionsDialog,
} from "@/features/messages/Reactions";
import { DeleteMessageModal } from "@/features/messages/DeleteMessageDialog";
import { messageLink, messageUrl, threadLink, type ChannelHome } from "@/features/messages/links";
import { LinkedMessages } from "@/features/messages/EmbeddedMessage";
import { WarningBody } from "@/features/messages/WarningBody";
import { ReportModal } from "@/features/reports/ReportDialog";
import { useAspenClient } from "@/api/context";
import { useMessages } from "@/i18n/context";
import { feelPress } from "@/features/messages/haptics";
import { MessageActionSheet } from "@/features/messages/MessageActionSheet";
import { MessageActions, type MessageSheet } from "@/features/messages/MessageActions";
import { TOUCH_ONLY, useMediaQuery } from "@/features/layout/useMediaQuery";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";
import { UserMention } from "@/features/messages/Mention";
import { formatNodes } from "@/i18n/formatNodes";
import { useNameIn } from "@/features/users/nameIn";
import { useRowProps } from "@/features/messages/messageRows";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };
/** The time a grouped message shows after what it ends with, its day being its group's. */
const TIME_OF_DAY: Intl.DateTimeFormatOptions = { timeStyle: "short" };

/** How long a finger is held on a message before its actions are offered under it. */
const LONG_PRESS_MS = 450;

/** What a long press has open: the actions, or what one of them opened in their place. */
type Sheet = "actions" | MessageSheet;

/** How far the pointer's actions rise above their message's top, over the message before. */
const TOOLBAR_RISE_PX = 20;

/** The room between the bottom of the pointer's actions and a grouped message's time under them. */
const TRAILING_CLEARANCE_PX = 2;

/** How soon after a message arrives that it is drawn arriving. */
const ARRIVING_MS = 1000;

/**
 * One message. `parentId` is the parent channel when `channelId` is a thread: its messages
 * link to the thread and cannot start threads of their own. Elsewhere a message offers to
 * start a thread, and one that started a thread shows its replies' summary; an echo shows the
 * thread reply it names.
 *
 * A message that continues its author's group (`grouped`, `messageGroups`) is drawn without
 * their picture, name, and time, under the message before it, which the list says is
 * `continued` by it; a little padding still parts the two. Its author and time are kept for
 * assistive technology, a pointer over it or focus in it shows its time after whatever it ends
 * with (`MessageBody`'s `trailing`), and a touch screen's actions say when it was sent.
 *
 * Its actions (`MessageActions`) show in a bar rising over its top corner while the pointer is
 * over it or focus is in it, kept out of the layout so the header and body sit where they
 * would without it; near the top of its list, the bar rises only as far as there is room. A
 * touch screen has no hover, and a row of buttons over every message would cost the
 * screen's room, so there a long press on the message opens them in a sheet sliding up from
 * the bottom (`MessageActionSheet`), with quick reactions above them and a tap felt in the
 * hand in the apps; the press owns the message, so the browser's own long press, which would
 * select its text, is turned off there, and the actions copy the text instead.
 */
export const MessageItem = memo(function MessageItem({
  id,
  home,
  channelId,
  parentId,
  highlighted,
  threadable = true,
  grouped = false,
  continued = false,
}: {
  id: string;
  home: ChannelHome;
  channelId: string;
  parentId: string | null;
  highlighted: boolean;
  /** Whether the message may start or show a thread here; not for a thread's own header. */
  threadable?: boolean;
  /** Whether it continues its author's group, drawn without picture, name, or time. */
  grouped?: boolean;
  /** Whether the message after it continues its group. */
  continued?: boolean;
}) {
  const m = useMessages();
  const sync = useSync();
  const client = useAspenClient();
  const navigate = useNavigate();
  const message = useMessage(id);
  const author = useUser(message?.author);
  const authorName = useNameIn(author, home.community);
  const authorLoading = useUserLoading(message?.author);
  const me = useMe();
  const permissions = useChannelAccess(channelId);
  const fullTime = useDateFormat(TIME);
  const timeOfDay = useDateFormat(TIME_OF_DAY);
  const [editing, setEditing] = useState(false);
  const rowProps = useRowProps(id);
  // How far the pointer's actions rise above the row: all the way, or as far as there is room.
  const [toolbarRise, setToolbarRise] = useState(TOOLBAR_RISE_PX);
  // How far under the top of what the body ends with its time and saved mark start, when they
  // sit beside it, so the actions never cover them (`MessageBody`'s `trailingDrop`).
  const [trailingDrop, setTrailingDrop] = useState<number | undefined>(undefined);
  const toolbar = useRef<HTMLDivElement>(null);
  // A touch screen offers the actions under a long press; a pointer, at the corner.
  const touchOnly = useMediaQuery(TOUCH_ONLY);
  // What the press has open: the actions, or what one of them opened in their place.
  const [sheet, setSheet] = useState<Sheet | null>(null);
  const row = useRef<HTMLElement>(null);
  // Whether the message has been pressed long, from when its sheets are drawn: they stay drawn
  // once it has, so each can slide away when it closes.
  const [pressed, setPressed] = useState(false);
  const { longPressProps } = useLongPress({
    isDisabled: !touchOnly || editing,
    threshold: LONG_PRESS_MS,
    accessibilityDescription: m.longPressForActions,
    // The reader's most used emoji are read as the finger goes down, so the quick reactions
    // have them by the time the press is long enough to show them.
    onLongPressStart: () => {
      if (
        permissions.has("addReactions") &&
        sync.store.frequentEmoji(home.community) === undefined
      ) {
        void sync.loadFrequentEmoji(home.community).catch(() => undefined);
      }
    },
    onLongPress: () => {
      feelPress();
      setPressed(true);
      setSheet("actions");
    },
  });
  const sheetProps = (which: Sheet) => ({
    isOpen: sheet === which,
    onOpenChange: (open: boolean) => {
      setSheet(open ? which : null);
    },
  });
  // A message that came while the reader was here rises into place; history arrives still.
  const [arriving] = useState(() => {
    const at = sync.store.arrivedAt(id);
    return at !== undefined && Date.now() - at < ARRIVING_MS ? "motion-rise " : "";
  });
  if (message === undefined) {
    return null;
  }
  const sentAt = new Date(message.timestamp);
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
  // The system account's notices and a call's record are no one's to report.
  const reportable =
    me !== null &&
    !own &&
    author?.system !== true &&
    message.kind !== "call" &&
    message.kind !== "missedCall";
  // A poll message has no text of its own; its card is edited by voting, not by rewriting.
  // Rewriting puts new words in it, which takes what posting here takes; deleting it does not.
  const editable =
    own &&
    message.kind === "standard" &&
    permissions.has(inThread ? "sendInThreads" : "sendMessages");
  // Its author may show a reply in the parent channel later, as the message box offers to as
  // it is sent, while it has no echo there.
  const echoParent =
    inThread &&
    own &&
    message.echo == null &&
    (message.kind === "standard" || message.kind === "command") &&
    permissions.has("sendMessages")
      ? parentId
      : null;
  if (message.kind === "call" || message.kind === "missedCall") {
    return (
      <NoticeRow id={id} arriving={arriving} highlighted={highlighted}>
        {message.kind === "call" ? (
          <CallNotice starter={message.author} seconds={message.callSeconds ?? 0} />
        ) : (
          <MissedCallNotice caller={message.author} />
        )}
      </NoticeRow>
    );
  }
  if (message.kind === "pollClosed" && message.poll != null) {
    return (
      <NoticeRow id={id} arriving={arriving} highlighted={highlighted}>
        <PollClosedNotice pollId={message.poll} home={home} channelId={channelId} />
      </NoticeRow>
    );
  }
  const actions = {
    messageId: id,
    channelId,
    communityId: home.community,
    text: message.content,
    permissions,
    canThread,
    echoParent,
    editable,
    deletable,
    reportable,
    link: messageUrl(client.baseUrl, home.community, channelId, id),
    onOpenThread: openThread,
    onEdit: () => {
      setEditing(true);
    },
  };
  // The pointer's actions rise over the message before, but no further than the top of the
  // list, or of whatever holds the row, where they would be cut off or cover what is above.
  // Its time and saved mark, beside the top of a picture or card, start under them where they
  // would reach across under them; elsewhere, and under the block where the line has no room
  // beside it, they are clear of the actions already.
  const placeToolbar = () => {
    const el = row.current;
    if (el === null) {
      return;
    }
    const room = el.closest<HTMLElement>("[data-message-list]") ?? el.parentElement;
    const top = el.getBoundingClientRect().top;
    const rise = Math.min(
      TOOLBAR_RISE_PX,
      Math.max(0, Math.floor(top - (room?.getBoundingClientRect().top ?? 0))),
    );
    setToolbarRise(rise);
    const trailing = el.querySelector<HTMLElement>(`[data-trailing="${CSS.escape(id)}"]`);
    const block = trailing?.previousElementSibling;
    if (trailing == null || block == null || toolbar.current === null) {
      setTrailingDrop(undefined);
      return;
    }
    const blockBox = block.getBoundingClientRect();
    const trailingBox = trailing.getBoundingClientRect();
    const toolbarBox = toolbar.current.getBoundingClientRect();
    // Where its line starts, whatever drop it has now: the block's top beside it, its bottom
    // under it.
    const lineTop = trailingBox.top - parseFloat(getComputedStyle(trailing).marginTop);
    const beside = lineTop < blockBox.bottom;
    // Only what reaches across under the actions needs to dodge them.
    const underToolbar = trailingBox.right > toolbarBox.left && trailingBox.left < toolbarBox.right;
    const toolbarBottom = top - rise + toolbarBox.height;
    setTrailingDrop(
      beside && underToolbar
        ? Math.max(0, toolbarBottom + TRAILING_CLEARANCE_PX - blockBox.top)
        : undefined,
    );
  };
  // A message that tags the reader stands out, with a bar at its edge in place of padding.
  const tagsMe = sync.store.mentionsMe(message);
  return (
    <article
      ref={row}
      data-message-id={id}
      data-mentions-me={tagsMe ? "true" : undefined}
      {...rowProps}
      {...(touchOnly
        ? longPressProps
        : {
            onPointerEnter: placeToolbar,
            onFocus: () => {
              rowProps.onFocus();
              placeToolbar();
            },
          })}
      className={
        "group relative flex gap-3 rounded-md outline-none focus-visible:ring-2 focus-visible:ring-accent/50 " +
        (grouped ? "pt-0.5 " : "pt-1.5 ") +
        (continued ? "pb-0.5 " : "pb-1.5 ") +
        (touchOnly ? "select-none [-webkit-touch-callout:none] " : "") +
        arriving +
        (highlighted ? "motion-flash " : "") +
        (tagsMe ? "border-s-2 border-accent pe-2 ps-1.5 " : "px-2 ") +
        (tagsMe
          ? "bg-accent-soft/50"
          : "hover:bg-surface-hover/60 focus-within:bg-surface-hover/60")
      }
    >
      {touchOnly && pressed && (
        <>
          <MessageActionSheet
            messageId={id}
            communityId={home.community}
            sentAt={fullTime.format(sentAt)}
            canReact={permissions.has("addReactions")}
            {...sheetProps("actions")}
            onMore={() => {
              setSheet("react");
            }}
          >
            <MessageActions
              {...actions}
              open={setSheet}
              onDone={() => {
                setSheet(null);
              }}
            />
          </MessageActionSheet>
          <ReactionPickerOverlay
            messageId={id}
            communityId={home.community}
            {...sheetProps("react")}
          />
          <ReactionsDialog
            messageId={id}
            communityId={home.community}
            {...sheetProps("reactions")}
          />
          <DeleteMessageModal messageId={id} {...sheetProps("delete")} />
          <ReportModal target={{ kind: "message", messageId: id }} {...sheetProps("report")} />
        </>
      )}
      {grouped ? (
        <div className="w-9 shrink-0" aria-hidden="true" />
      ) : author === undefined ? (
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
            aria-label={format(m.profile.show, { name: authorName ?? displayNameOf(author) })}
            excludeFromTabOrder
            className="h-fit shrink-0 rounded-full outline-none pressed:opacity-80 focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <Avatar name={authorName ?? displayNameOf(author)} iconId={author.icon} />
          </Button>
        </ProfilePopover>
      )}
      <div className="min-w-0 flex-1">
        {grouped ? (
          <span className="sr-only">
            {authorName ?? (author === undefined ? m.unknownUser : displayNameOf(author))}{" "}
            <time dateTime={message.timestamp}>{fullTime.format(sentAt)}</time>
          </span>
        ) : (
          <MessageHeader
            message={message}
            author={author}
            authorLoading={authorLoading}
            home={home}
            channelId={channelId}
            parentId={parentId}
          />
        )}
        {!editing && !touchOnly && (
          // Out of the header's flow, so its buttons never make the header taller than its
          // text; after the header in the document, so a keyboard reaches it before the body.
          // Hidden, it lets the pointer through to the message it rises over.
          <div
            role="group"
            aria-label={m.messageActionsLabel}
            ref={toolbar}
            className="pointer-events-none absolute end-2 z-10 flex gap-0.5 rounded-lg border border-line bg-surface-raised p-0.5 opacity-0 shadow-sm group-focus-within:pointer-events-auto group-focus-within:opacity-100 group-hover:pointer-events-auto group-hover:opacity-100"
            style={{ top: -toolbarRise }}
          >
            <MessageActions {...actions} />
          </div>
        )}
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
        {message.kind === "warning" && !editing ? (
          <WarningBody message={message} home={home} />
        ) : (
          <MessageBody
            message={message}
            home={home}
            hideText={editing || (message.kind === "threadEcho" && message.echoOf != null)}
            {...(trailingDrop === undefined ? {} : { trailingDrop })}
            {...(grouped && !touchOnly
              ? {
                  // Read out with the author above, so shown alone.
                  trailing: (
                    <time
                      aria-hidden="true"
                      dateTime={message.timestamp}
                      title={fullTime.format(sentAt)}
                      className="hidden text-xs whitespace-nowrap text-ink-faint group-focus-within:inline group-hover:inline"
                    >
                      {timeOfDay.format(sentAt)}
                    </time>
                  ),
                }
              : {})}
            {...(own || permissions.has("manageMessages")
              ? {
                  onRemoveAttachment: (attachmentId: string) => {
                    void sync.removeAttachment(id, attachmentId).catch(() => undefined);
                  },
                }
              : {})}
          />
        )}
        {!editing && <LinkedMessages message={message} />}
        {!editing && <MessageAnnotations messageId={id} />}
        <ReactionChips
          messageId={id}
          canReact={permissions.has("addReactions")}
          communityId={home.community}
        />
        {canThread && message.thread != null && (
          <ThreadSummary threadId={message.thread} home={home} channelId={channelId} />
        )}
      </div>
    </article>
  );
});

/** A notice the server writes into the conversation, under no author, set in line with text. */
function NoticeRow({
  id,
  arriving,
  highlighted,
  children,
}: {
  id: string;
  /** The arriving motion's class, or nothing for a message drawn still. */
  arriving: string;
  highlighted: boolean;
  children: ReactNode;
}) {
  const rowProps = useRowProps(id);
  return (
    <article
      data-message-id={id}
      {...rowProps}
      className={
        "flex gap-3 rounded-md px-2 py-1.5 outline-none focus-visible:ring-2 focus-visible:ring-accent/50 " +
        arriving +
        (highlighted ? "motion-flash " : "") +
        "hover:bg-surface-hover/60"
      }
    >
      <div className="w-10 shrink-0" aria-hidden="true" />
      <div className="min-w-0 flex-1">{children}</div>
    </article>
  );
}

/**
 * A message's first line: who wrote it, what they are, what kind of message it is, and when,
 * linking to the message. It holds only text, so it is as tall as a line of it.
 */
function MessageHeader({
  message,
  author,
  authorLoading,
  home,
  channelId,
  parentId,
}: {
  message: Message;
  author: User | undefined;
  authorLoading: boolean;
  home: ChannelHome;
  channelId: string;
  parentId: string | null;
}) {
  const timeFormat = useDateFormat(TIME);
  const m = useMessages();
  const nameColor = useNameColor(author?.id, home.community);
  const name = useNameIn(author, home.community);
  // A thread's messages link to the thread; elsewhere a message links to itself in its history.
  const permalink =
    parentId === null
      ? messageLink(home, channelId, message.id)
      : threadLink(home, parentId, channelId);
  return (
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
            aria-label={format(m.profile.show, { name: name ?? displayNameOf(author) })}
            className="rounded font-medium outline-none hover:underline focus-visible:ring-2 focus-visible:ring-accent/50"
            style={{ color: nameColor }}
          >
            {name ?? displayNameOf(author)}
          </Button>
        </ProfilePopover>
      )}
      {author?.bot === true && <BotBadge />}
      {author?.system === true && <SystemBadge />}
      {message.kind === "command" && message.commandBot != null && (
        <span className="flex items-center gap-1 text-xs text-ink-muted">
          <RobotIcon size={12} aria-hidden="true" />
          {formatNodes(m.commands.sentTo, {
            bot: <UserMention id={message.commandBot} chip communityId={home.community} />,
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
        <time dateTime={message.timestamp}>{timeFormat.format(new Date(message.timestamp))}</time>
      </Link>
    </div>
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
      <MessageMedia attachmentIds={reply.attachments} previewImages={[]} />
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
