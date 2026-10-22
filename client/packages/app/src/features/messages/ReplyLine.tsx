import type { Message } from "@aspen/protocol";
import { ChartBarIcon, ImageIcon } from "@phosphor-icons/react";
import {
  useLinkedMessage,
  useMessageOnDemand,
  useNickname,
  useStore,
  useUser,
  useUserLoading,
} from "@/api/hooks";
import { Avatar } from "@/features/communities/Avatar";
import { decodeCustomEmoji } from "@/features/emoji/customEmoji";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { decodeTags } from "@/features/mentions/tags";
import { useJumpToMessage } from "@/features/messages/jumpToMessage";
import type { ChannelHome } from "@/features/messages/links";
import { replyPreview } from "@/features/messages/replyPreview";
import { useNameColor } from "@/features/users/nameColor";
import { displayNameOf } from "@/features/users/profile";
import { useMessages } from "@/i18n/context";

/**
 * Where the line meets the message's picture: a bend from the picture's middle, in the column
 * the picture takes (`w-11`), up and across to the line, as a thread of conversation drawn
 * between the two.
 */
const SPINE_CLASS =
  "ms-[1.3125rem] mt-2 h-2 w-[1.6875rem] shrink-0 self-start rounded-ss-md border-s-2 border-t-2 border-line";

/**
 * Above a reply, the message it answers: its author's picture and name (with `@` before it when
 * the reply tagged them), and a line of its text, which jumps to it in the list
 * (`useJumpToMessage`). Since a reply answers a message of its own channel, whoever reads the
 * reply may read the original, which comes with the history the reply was read in, or is read
 * on its own when the reply arrived since. A deleted original is said to be, as the read of
 * the reply's links (`linked`) or the original's own read finds.
 *
 * It is a plain `button`, not React Aria's: a list draws hundreds of rows, and a row holds as
 * little as it can; a click is all it takes, from a pointer, a finger, or a key.
 */
export function ReplyLine({
  message,
  replyTo,
  home,
}: {
  message: Message;
  /** The message it answers, its `replyTo`. */
  replyTo: string;
  home: ChannelHome;
}) {
  const m = useMessages();
  const link = useLinkedMessage(replyTo);
  const gone = link !== undefined && link.state !== "available";
  const { message: original, missing } = useMessageOnDemand(replyTo, !gone);
  let shown;
  if (gone || missing) {
    shown = (
      <span className="truncate text-ink-faint italic">
        {link?.state === "unavailable" ? m.replies.unavailable : m.replies.deleted}
      </span>
    );
  } else if (original === undefined) {
    shown = (
      <span aria-busy="true" className="flex items-center gap-1.5">
        <LoadingLabel text={m.replies.loading} />
        <Skeleton className="h-5 w-5 shrink-0 rounded-full" />
        <Skeleton inline className="w-48" />
      </span>
    );
  } else {
    shown = <Original reply={message} original={original} home={home} />;
  }
  return (
    <div className="flex min-w-0 basis-full items-center gap-1.5 text-sm text-ink-muted">
      <span aria-hidden="true" className={SPINE_CLASS} />
      {shown}
    </div>
  );
}

/** The original as the line shows it, jumping to it when pressed. */
function Original({
  reply,
  original,
  home,
}: {
  reply: Message;
  original: Message;
  home: ChannelHome;
}) {
  const m = useMessages();
  const store = useStore();
  const jump = useJumpToMessage();
  const author = useUser(original.author);
  const authorLoading = useUserLoading(original.author);
  const nickname = useNickname(home.community, original.author);
  const color = useNameColor(original.author, home.community);
  const name =
    author === undefined
      ? authorLoading
        ? null
        : m.unknownUser
      : (nickname ?? displayNameOf(author));
  // Tags read as names, as the message shows them; one whose record is not here reads as a tag
  // with no name rather than as its id.
  const roles = home.community === null ? [] : store.roles(home.community);
  const { text } = decodeTags(
    original.content,
    (id) => store.user(id)?.name ?? "…",
    (id) => roles.find((role) => role.id === id)?.name ?? "…",
  );
  const emoji = home.community === null ? [] : store.customEmoji(home.community);
  const preview = replyPreview(decodeCustomEmoji(text, emoji), m.replies.spoiler);
  const tagged = reply.mentions.users.includes(original.author);
  const contents = (
    <>
      <span className="sr-only">{m.replies.replyingTo}</span>
      {author === undefined ? (
        <Skeleton className="h-5 w-5 shrink-0 rounded-full" />
      ) : (
        <Avatar name={name ?? ""} iconId={author.icon} size="xs" />
      )}
      {name === null ? (
        <Skeleton inline className="w-16" />
      ) : (
        <span className="max-w-[45%] shrink-0 truncate font-medium" style={{ color }}>
          {tagged ? `@${name}` : name}
        </span>
      )}
      {preview !== "" ? (
        <span className="truncate">{preview}</span>
      ) : (
        <span className="flex min-w-0 items-center gap-1 text-ink-faint italic">
          {original.kind === "poll" ? (
            <ChartBarIcon size={14} aria-hidden="true" className="shrink-0" />
          ) : original.attachments.length > 0 ? (
            <ImageIcon size={14} aria-hidden="true" className="shrink-0" />
          ) : null}
          <span className="truncate">
            {original.kind === "poll"
              ? m.replies.poll
              : original.attachments.length > 0
                ? m.replies.attachment
                : m.replies.openOriginal}
          </span>
        </span>
      )}
    </>
  );
  const lineClass = "flex min-w-0 items-center gap-1.5 rounded text-start";
  if (jump === null) {
    return <span className={lineClass}>{contents}</span>;
  }
  return (
    <button
      type="button"
      onClick={() => {
        jump(original.channelId, original.id);
      }}
      className={
        lineClass +
        " cursor-pointer outline-none hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
      }
    >
      {contents}
    </button>
  );
}
