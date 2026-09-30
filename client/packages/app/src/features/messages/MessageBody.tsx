import type { LinkPreview, Message } from "@aspen/protocol";
import { MessageMedia } from "@/features/messages/Attachments";
import { imageUrls, isImageUrl, onlyImageLinks } from "@/features/messages/images";
import type { ChannelHome } from "@/features/messages/links";
import { Markdown } from "@/features/messages/Markdown";
import { PollCard } from "@/features/messages/PollCard";
import { VideoCard } from "@/features/messages/VideoCard";
import { playerSrc } from "@/features/messages/video";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/**
 * What a message says and holds, drawn the same wherever it is shown (the channel, the pins
 * list): its text as Markdown with its tags, marked when edited, its poll, its pictures and
 * files, and cards for its links. A message that is nothing but links to pictures shows the
 * pictures alone. `hideText` leaves the text out where something takes its place (the editor,
 * an echo's reply); `onRemoveAttachment` offers each attachment's removal to those who may.
 */
export function MessageBody({
  message,
  home,
  hideText = false,
  onRemoveAttachment,
}: {
  message: Message;
  home: ChannelHome;
  hideText?: boolean;
  onRemoveAttachment?: (attachmentId: string) => void;
}) {
  const m = useMessages();
  const timeFormat = useDateFormat(TIME);
  const linkedImages = imageUrls(message.content);
  // A link the client already shows as a picture needs no card from the server for the same URL.
  const previews = message.linkPreviews.filter((p) => !linkedImages.includes(p.url));
  // Links the server found to be images themselves join the message's pictures; the rest
  // are cards.
  const previewImages = previews.flatMap((p) =>
    isPictureOnly(p) && p.imageUrl != null ? [{ src: p.imageUrl, name: p.url }] : [],
  );
  const cards = previews.filter((p) => !isPictureOnly(p));
  const pictureOnly = onlyImageLinks(
    message.content,
    (url) => isImageUrl(url) || previews.some((p) => p.url === url && isPictureOnly(p)),
  );
  return (
    <>
      {!hideText && (
        <div className="flex flex-wrap items-baseline gap-x-1">
          {!pictureOnly && (
            <Markdown
              content={message.content}
              mentions={message.mentions}
              communityId={home.community}
            />
          )}
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
        {...(onRemoveAttachment === undefined ? {} : { onRemove: onRemoveAttachment })}
      />
      {cards.map((preview) => (
        <LinkPreviewCard key={preview.url} preview={preview} />
      ))}
    </>
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
