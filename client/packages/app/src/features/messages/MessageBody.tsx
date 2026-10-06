import type { LinkPreview, Message } from "@aspen/protocol";
import { useLayoutEffect, useState } from "react";
import { flushSync } from "react-dom";
import { MessageMedia } from "@/features/messages/Attachments";
import { imageUrls, keptRoom, onlyImageLinks } from "@/features/messages/images";
import { useKeepStill } from "@/features/messages/keepStill";
import type { ChannelHome } from "@/features/messages/links";
import { Markdown } from "@/features/messages/Markdown";
import { PollCard } from "@/features/messages/PollCard";
import { AlteredBy } from "@/features/plugins/Annotations";
import { PluginCard } from "@/features/plugins/PluginCard";
import { VideoCard } from "@/features/messages/VideoCard";
import { playerSrc } from "@/features/messages/video";
import { mediaUrl, webPageUrl } from "@/features/layout/safeUrl";
import { useMessages } from "@/i18n/context";
import { useDateFormat } from "@/i18n/format";
import { format } from "@/i18n/messages";

const TIME: Intl.DateTimeFormatOptions = { dateStyle: "medium", timeStyle: "short" };

/**
 * What a message says and holds, drawn the same wherever it is shown (the channel, the pins
 * list): its text as Markdown with its tags, marked when edited or changed by a plugin, its
 * poll or plugin's card, its pictures and files, and cards for its links. A message that is
 * nothing but links to pictures the server has previews of shows the pictures alone. `hideText` leaves the text out where something takes its place (the editor,
 * an echo's reply); `onRemoveAttachment` offers each attachment's removal to those who may.
 * `still` draws it for reference only, as another message shows it: no poll to vote in, no
 * card's buttons to press.
 */
export function MessageBody({
  message,
  home,
  hideText = false,
  still = false,
  onRemoveAttachment,
}: {
  message: Message;
  home: ChannelHome;
  hideText?: boolean;
  still?: boolean;
  onRemoveAttachment?: (attachmentId: string) => void;
}) {
  const m = useMessages();
  const timeFormat = useDateFormat(TIME);
  const imageLinks = imageUrls(message.content);
  // Every picture is the server's copy, kept in its storage, of what it found behind a link,
  // never the link itself: loading a picture from wherever a message points would tell whoever
  // serves it the address of everyone who reads the message. A preview whose picture is not a
  // file a deployment may serve (`mediaUrl`) is taken as having none.
  const linkPreviews = message.linkPreviews.map((p) => ({
    ...p,
    imageUrl: mediaUrl(p.imageUrl) ?? null,
  }));
  // A link that looks like a picture by its extension is drawn from the preview's picture: a
  // share page named like a gif (tenor.com/….gif) is a page, and only the preview's picture,
  // its og:image, is the gif. Until the preview comes, or when it has no picture, the link
  // stays a link in the text.
  const found = new Map(
    linkPreviews.flatMap((p) => (p.imageUrl == null ? [] : [[p.url, p] as const])),
  );
  const foundImages = imageLinks.flatMap((url) => {
    const p = found.get(url);
    return p?.imageUrl == null
      ? []
      : [{ src: p.imageUrl, name: url, width: p.imageWidth, height: p.imageHeight }];
  });
  // A link already shown as a picture needs no card for the same URL.
  const previews = linkPreviews.filter((p) => !(imageLinks.includes(p.url) && found.has(p.url)));
  // Links the server found to be images themselves join the message's pictures; the rest
  // are cards.
  const previewImages = [
    ...foundImages,
    ...previews.flatMap((p) =>
      isPictureOnly(p) && p.imageUrl != null
        ? [{ src: p.imageUrl, name: p.url, width: p.imageWidth, height: p.imageHeight }]
        : [],
    ),
  ];
  const cards = previews.filter((p) => !isPictureOnly(p));
  const pictureOnly = onlyImageLinks(message.content, (url) =>
    previewImages.some((picture) => picture.name === url),
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
          <AlteredBy pluginIds={message.alteredBy} />
        </div>
      )}
      {!still && message.kind === "poll" && message.poll != null && (
        <PollCard pollId={message.poll} />
      )}
      <PluginCard message={message} still={still} />
      <MessageMedia
        attachmentIds={message.attachments}
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
