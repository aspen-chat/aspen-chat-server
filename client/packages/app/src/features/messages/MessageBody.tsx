import type { LinkPreview, Message } from "@aspen/protocol";
import { useLayoutEffect, useState, type ReactNode } from "react";
import { flushSync } from "react-dom";
import { MessageMedia } from "@/features/messages/Attachments";
import { keptRoom } from "@/features/messages/images";
import { useKeepStill } from "@/features/messages/keepStill";
import { linkPictures } from "@/features/messages/linkPictures";
import type { ChannelHome } from "@/features/messages/links";
import { Markdown } from "@/features/messages/Markdown";
import { PollCard } from "@/features/messages/PollCard";
import { SpoilerCard } from "@/features/messages/SpoilerCard";
import { AlteredBy } from "@/features/plugins/Annotations";
import { useCardPlugin } from "@/features/plugins/cardPlugin";
import { PluginCard } from "@/features/plugins/PluginCard";
import { VideoCard } from "@/features/messages/VideoCard";
import { playerSrc } from "@/features/messages/video";
import { ErrorBoundary } from "@/features/layout/ErrorBoundary";
import { mediaUrl, webPageUrl } from "@/features/layout/safeUrl";
import { useIsSaved, useUser } from "@/api/hooks";
import { BookmarkSimpleIcon } from "@phosphor-icons/react";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/**
 * What a message says and holds, drawn the same wherever it is shown (the channel, the pins
 * list): its text as Markdown with its tags, marked when edited or changed by a plugin, its
 * poll or plugin's card, its pictures and files, and cards for its links. A message that is
 * nothing but links to pictures the server has previews of shows the pictures alone.
 * `hideText` leaves the text out where something takes its place (the editor, an echo's
 * reply); `onRemoveAttachment`, for those who may, puts Remove attachment in each attachment's
 * menu, which asks before it is removed. `still` draws it for reference only, as another message
 * shows it: no poll to vote in, no card's buttons to press. The reader's mark on a message they
 * saved, then `trailing`, follow whatever the message ends with: its text, after its edited
 * mark, or beside its last card or its pictures where the line has room, and under them where it
 * has not (`Trailed`). After the text, the marks end its last line, as a word would, wrapping
 * under it where the line has no room, and sit beside a code block, table, or rule it ends
 * with, under it only where the line has no room (`Markdown`'s `after`). A message that fails to
 * draw shows its text alone, plainly, rather than taking the list it is in down with it. Without
 * `savedMark` the saved mark is left to whoever draws the message, as `MessageItem` gives it to
 * its header on a touch screen.
 */
export function MessageBody(props: MessageBodyProps) {
  const { message, hideText = false } = props;
  return (
    <ErrorBoundary
      resetKey={message}
      fallback={
        hideText ? null : <p className="message-body whitespace-pre-wrap">{message.content}</p>
      }
    >
      <MessageBodyContent {...props} />
    </ErrorBoundary>
  );
}

interface MessageBodyProps {
  message: Message;
  home: ChannelHome;
  hideText?: boolean;
  still?: boolean;
  onRemoveAttachment?: (attachmentId: string, name: string) => void;
  trailing?: ReactNode;
  /** Whether the reader's mark on a message they saved trails it; it does by default. */
  savedMark?: boolean;
  /**
   * How far below the top of the block it trails the saved mark and `trailing` start, in
   * pixels, while they sit beside it, to clear what covers that corner (`MessageItem`'s
   * actions); level with the block's top edge without it.
   */
  trailingDrop?: number;
}

function MessageBodyContent({
  message,
  home,
  hideText = false,
  still = false,
  onRemoveAttachment,
  trailing,
  savedMark = true,
  trailingDrop,
}: MessageBodyProps) {
  const m = useMessages();
  const timeFormat = useDateFormat(TIME);
  // The system account's notices quote names others chose (a community's, a person's), so
  // nothing in them is a link the deployment would seem to vouch for.
  const fromSystem = useUser(message.author)?.system === true;
  // Every picture is the server's copy, kept in its storage, of what it found behind a link,
  // never the link itself: loading a picture from wherever a message points would tell whoever
  // serves it the address of everyone who reads the message. A preview whose picture is not a
  // file a deployment may serve (`mediaUrl`) is taken as having none.
  const {
    pictures: previewImages,
    cards,
    isSpoilered,
    pictureOnly,
  } = linkPictures(
    message.content,
    message.linkPreviews.map((p) => ({ ...p, imageUrl: mediaUrl(p.imageUrl) ?? null })),
  );
  const pollId = still || message.kind !== "poll" ? null : (message.poll ?? null);
  const pluginCard = useCardPlugin(message) !== undefined;
  const media = message.attachments.length > 0 || previewImages.length > 0;
  const last =
    cards.length > 0
      ? "cards"
      : media
        ? "media"
        : pluginCard
          ? "pluginCard"
          : pollId !== null
            ? "poll"
            : "text";
  const saved = useIsSaved(message.id) && savedMark;
  const tail =
    saved || trailing !== undefined ? (
      <>
        {saved && <SavedMark />}
        {trailing}
      </>
    ) : undefined;
  const trailingAfter = (block: typeof last) => (block === last ? tail : undefined);
  const afterText =
    message.editedAt != null ||
    message.alteredBy.length > 0 ||
    trailingAfter("text") !== undefined ? (
      <>
        {message.editedAt != null && (
          <span
            className="text-xs text-ink-faint"
            title={format(m.editedAt, { date: timeFormat.format(new Date(message.editedAt)) })}
          >
            {m.edited}
          </span>
        )}
        <AlteredBy pluginIds={message.alteredBy} />
        {trailingAfter("text")}
      </>
    ) : null;
  const trailed = (block: typeof last) => ({
    trailing: trailingAfter(block),
    drop: trailingDrop,
    messageId: message.id,
  });
  return (
    <>
      {!hideText && (
        <div className="flex flex-wrap items-baseline gap-x-1">
          {pictureOnly ? (
            afterText
          ) : (
            <Markdown
              content={message.content}
              mentions={message.mentions}
              communityId={home.community}
              links={!fromSystem}
              after={afterText}
            />
          )}
        </div>
      )}
      {pollId !== null && (
        <Trailed {...trailed("poll")} card>
          <PollCard pollId={pollId} />
        </Trailed>
      )}
      <Trailed {...trailed("pluginCard")} card>
        <PluginCard message={message} still={still} />
      </Trailed>
      <Trailed {...trailed("media")}>
        <MessageMedia
          attachmentIds={message.attachments}
          previewImages={previewImages}
          {...(onRemoveAttachment === undefined ? {} : { onRemove: onRemoveAttachment })}
        />
      </Trailed>
      {cards.map((preview, i) => (
        <Trailed
          key={preview.url}
          {...trailed("cards")}
          trailing={i === cards.length - 1 ? trailingAfter("cards") : undefined}
          card
        >
          {isSpoilered(preview) ? (
            <SpoilerCard>
              <LinkPreviewCard preview={preview} />
            </SpoilerCard>
          ) : (
            <LinkPreviewCard preview={preview} />
          )}
        </Trailed>
      ))}
    </>
  );
}

/**
 * A block of a message with what trails it (its saved mark and `MessageBodyProps.trailing`),
 * kept together, beside its top where the line has room, so a tall picture does not leave it
 * far from the text above, and under the block where it has not. A `card` keeps the width it
 * has alone (`w-full max-w-lg`, so as wide as the line up to 32rem) and gives way only when the
 * line is narrower than that and the trailing together; anything else is as wide as what it
 * holds. Beside the block, the trailing starts `drop` pixels below its top edge where given;
 * under it, it keeps the gap every block opens with. It is marked with its message's id
 * (`data-trailing`), by which `MessageItem` finds and measures it. Without `trailing` the
 * block is drawn as it is.
 */
function Trailed({
  trailing,
  drop,
  messageId,
  card = false,
  children,
}: {
  trailing: ReactNode;
  drop: number | undefined;
  messageId: string;
  card?: boolean;
  children: ReactNode;
}) {
  if (trailing === undefined) {
    return children;
  }
  return (
    <div className="flex flex-wrap items-start gap-x-2">
      <div className={card ? "min-w-0 flex-[0_1_32rem]" : "min-w-0"}>{children}</div>
      {/* `mt-1` is level with the block's top edge, below the margin every block opens with. */}
      <span
        data-trailing={messageId}
        className="mt-1 flex items-center gap-x-1"
        style={drop === undefined ? undefined : { marginTop: drop }}
      >
        {trailing}
      </span>
    </div>
  );
}

function LinkPreviewCard({ preview }: { preview: LinkPreview }) {
  const player = preview.video == null ? null : playerSrc(preview.video.src);
  if (preview.video != null && player !== null) {
    return <VideoCard preview={preview} player={player} />;
  }
  const title = preview.title ?? preview.siteName ?? preview.url;
  return (
    <a
      href={webPageUrl(preview.url)}
      target="_blank"
      rel="noreferrer"
      className="mt-1 flex max-w-lg flex-col gap-2 rounded-md border border-line bg-surface-raised p-2 text-sm hover:bg-surface-hover"
      style={
        preview.themeColor != null
          ? { borderLeftColor: preview.themeColor, borderLeftWidth: 3 }
          : undefined
      }
    >
      <span className="min-w-0">
        <span className="block font-medium wrap-anywhere text-accent">{title}</span>
        {preview.title != null && preview.siteName != null && (
          <span className="block wrap-anywhere text-ink-muted">{preview.siteName}</span>
        )}
        {preview.description != null && (
          <span className="mt-0.5 line-clamp-2 block text-ink-muted">{preview.description}</span>
        )}
      </span>
      {preview.imageUrl != null && (
        <CardPicture
          src={preview.imageUrl}
          width={preview.imageWidth}
          height={preview.imageHeight}
        />
      )}
    </a>
  );
}

/**
 * A link card's picture, beneath its text: as wide as the picture is or as the card is,
 * whichever is narrower, and no taller than a message's pictures (`max-h-80`), at its own
 * proportions. One whose size is known keeps exactly its room while it loads (`keptRoom`); one
 * whose size is not takes none until it has arrived, and then tells the list in the same task
 * (`useKeepStill`), so the view stays still as the row grows. One that fails to load is left
 * out.
 */
function CardPicture({
  src,
  width,
  height,
}: {
  src: string;
  width: number | null | undefined;
  height: number | null | undefined;
}) {
  const keepStill = useKeepStill();
  const [state, setState] = useState<"waiting" | "arrived" | "failed">("waiting");
  useLayoutEffect(() => {
    keepStill();
  }, [state, keepStill]);
  if (state === "failed") {
    return null;
  }
  const size = width != null && height != null ? { width, height } : undefined;
  return (
    <img
      src={src}
      alt=""
      loading="lazy"
      draggable={false}
      referrerPolicy="no-referrer"
      {...(size ?? {})}
      style={size === undefined ? undefined : keptRoom(size.width, size.height)}
      onLoad={() => {
        flushSync(() => {
          setState("arrived");
        });
      }}
      onError={() => {
        flushSync(() => {
          setState("failed");
        });
      }}
      className={
        "block h-auto max-h-80 max-w-full self-start rounded object-contain" +
        (state === "waiting"
          ? " animate-pulse bg-surface-hover motion-reduce:animate-none"
          : " bg-surface-sunken")
      }
    />
  );
}

/**
 * A quiet mark on a message the reader saved, which their saved messages list. The icon sits
 * in a line of text, so the mark has its line's baseline and lines up with the marks beside it.
 * `className` sets its size where it follows smaller text than the message's.
 */
export function SavedMark({ className = "" }: { className?: string }) {
  const m = useMessages();
  return (
    <span className={`leading-none text-ink-faint ${className}`} title={m.saved.marker}>
      <BookmarkSimpleIcon
        size="1em"
        weight="fill"
        aria-hidden="true"
        className="inline-block align-[-0.125em]"
      />
      <span className="sr-only">{m.saved.marker}</span>
    </span>
  );
}
