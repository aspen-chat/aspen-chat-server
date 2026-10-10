import { plainFileName, type Attachment } from "@aspen/protocol";
import { PaperclipIcon, PlayIcon, XIcon } from "@phosphor-icons/react";
import { useLayoutEffect, useState } from "react";
import { flushSync } from "react-dom";
import { Button } from "react-aria-components";
import { useAttachments, useStore } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { ImageGallery } from "@/features/messages/ImageGallery";
import {
  inlinePreview,
  isImageType,
  isVideoType,
  keptRoom,
  pictureAlt,
  splitInline,
  type InlinePreview,
  type Picture,
} from "@/features/messages/images";
import { useKeepStill } from "@/features/messages/keepStill";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { CopyIdButton } from "@/features/layout/CopyId";
import { mediaUrl, webPageUrl } from "@/features/layout/safeUrl";
import { OnceOpen } from "@/features/layout/OnceOpen";

const imageClass = "block max-h-80 max-w-full rounded-md border border-line object-contain";

/**
 * What a message carries besides its text: uploaded attachments, shown inline when they are
 * images, as their posters when they are videos the server took one of, and as download chips
 * otherwise, then the pictures the server found behind links in the text, each from its copy
 * in storage. An attachment whose address is not one a deployment may serve (`mediaUrl`) is
 * unavailable. Up to three pictures show inline; with more, a button in their place
 * opens the whole set in a gallery, and so does any picture. Inline, a picture shows the
 * smaller copy the server made of it where there is one; the gallery shows the original. With
 * `onRemove`, each attachment shown carries a control that takes it off the message.
 */
export function MessageMedia({
  attachmentIds,
  previewImages,
  onRemove,
}: {
  attachmentIds: readonly string[];
  /** Pictures the server found behind links, as `{ src, name }` with the link as the name. */
  previewImages: readonly Picture[];
  /** Takes an attachment off the message, for its author and those who manage messages. */
  onRemove?: (attachmentId: string) => void;
}) {
  const m = useMessages();
  const attachments = useAttachments(attachmentIds);
  const store = useStore();
  const [gallery, setGallery] = useState<number | null>(null);
  if (attachmentIds.length === 0 && previewImages.length === 0) {
    return null;
  }
  const pictures: Picture[] = [];
  const files: Attachment[] = [];
  const videos: Attachment[] = [];
  const unavailable: string[] = [];
  const coming: string[] = [];
  attachments.forEach((record, i) => {
    const id = attachmentIds[i] ?? "";
    if (record === undefined) {
      (store.missing("attachment", id) ? unavailable : coming).push(id);
      return;
    }
    const downloadUrl = mediaUrl(record.downloadUrl);
    if (downloadUrl === undefined) {
      unavailable.push(id);
      return;
    }
    // The name is the uploader's, shown without what would change how it reads.
    const attachment = { ...record, downloadUrl, fileName: plainFileName(record.fileName) };
    if (isImageType(attachment.mimeType)) {
      pictures.push({
        src: attachment.downloadUrl,
        name: attachment.fileName,
        attachmentId: attachment.id,
        width: attachment.width,
        height: attachment.height,
        description: attachment.description,
        preview: inlinePreview(attachment.preview),
      });
    } else if (isVideoType(attachment.mimeType) && attachment.preview != null) {
      videos.push(attachment);
    } else {
      files.push(attachment);
    }
  });
  pictures.push(...previewImages);
  const { shown, hidden } = splitInline(pictures);
  return (
    <ul aria-label={m.attachmentsLabel} className="mt-1 flex flex-wrap items-start gap-2">
      {coming.map((id) => (
        // Its record says whether it is a picture or a file, and how big; until it comes, a
        // picture's worth of room is the conservative guess.
        <li key={id} aria-busy="true" data-attachment-id={id}>
          <LoadingLabel />
          <Skeleton className="h-48 w-64 max-w-full" />
        </li>
      ))}
      {unavailable.map((id) => (
        <li key={id} className="text-sm text-ink-faint" data-attachment-id={id}>
          {m.attachmentUnavailable}
        </li>
      ))}
      {files.map((attachment) => (
        <li
          key={attachment.id}
          data-attachment-id={attachment.id}
          className="flex items-center gap-1"
        >
          <FileChip attachment={attachment} />
          {onRemove !== undefined && (
            <RemoveButton
              name={attachment.fileName}
              onPress={() => {
                onRemove(attachment.id);
              }}
            />
          )}
          <CopyIdButton id={attachment.id} thing="attachment" />
        </li>
      ))}
      {videos.map((attachment) => (
        <li
          key={attachment.id}
          data-attachment-id={attachment.id}
          className="relative min-w-0 max-w-full"
        >
          <InlineVideo attachment={attachment} />
          <span className="absolute top-1 end-1 flex gap-1">
            {onRemove !== undefined && (
              <RemoveButton
                name={attachment.fileName}
                onPress={() => {
                  onRemove(attachment.id);
                }}
              />
            )}
            <CopyIdButton
              id={attachment.id}
              thing="attachment"
              className="bg-surface-raised/90 shadow-sm"
            />
          </span>
        </li>
      ))}
      {shown.map((picture, i) => (
        <li key={picture.src + String(i)} className="relative min-w-0 max-w-full">
          <InlineImage
            picture={picture}
            onOpen={() => {
              setGallery(i);
            }}
          />
          {picture.attachmentId !== undefined && (
            <span className="absolute top-1 end-1 flex gap-1">
              {onRemove !== undefined && (
                <RemoveButton
                  name={picture.name}
                  onPress={() => {
                    if (picture.attachmentId !== undefined) {
                      onRemove(picture.attachmentId);
                    }
                  }}
                />
              )}
              <CopyIdButton
                id={picture.attachmentId}
                thing="attachment"
                className="bg-surface-raised/90 shadow-sm"
              />
            </span>
          )}
        </li>
      ))}
      {hidden > 0 && (
        <li>
          <Button
            onPress={() => {
              setGallery(shown.length);
            }}
            aria-label={format(m.viewAllImages, { count: String(pictures.length) })}
            className="flex h-32 w-32 items-center justify-center rounded-md border border-line bg-surface-sunken text-lg font-semibold text-ink-muted outline-none hover:bg-surface-hover pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            {format(m.moreImages, { count: String(hidden) })}
          </Button>
        </li>
      )}
      <OnceOpen isOpen={gallery !== null}>
        <ImageGallery
          pictures={pictures}
          initial={gallery ?? 0}
          isOpen={gallery !== null}
          onClose={() => {
            setGallery(null);
          }}
        />
      </OnceOpen>
    </ul>
  );
}

/**
 * A picture in the message; pressing it opens the message's gallery on that picture. It shows
 * the server's smaller copy where there is one, at the copy's exact size, and the original if
 * the copy cannot be loaded; an original already loaded stays when a copy arrives later, as its
 * bytes are already here. A picture whose size is known keeps exactly its room while it loads
 * (`keptRoom`). One whose size is not keeps a fixed square, a conservative guess, until it has
 * arrived, and then takes its own size: rendered as it arrives and told to the list in the same
 * task (`useKeepStill`), so the list keeps the view still through the change before anything
 * else runs. Either pulses as a skeleton until it has loaded.
 */
function InlineImage({ picture, onOpen }: { picture: Picture; onOpen: () => void }) {
  const m = useMessages();
  const keepStill = useKeepStill();
  const [arrived, setArrived] = useState<string | null>(null);
  const [failed, setFailed] = useState<ReadonlySet<string>>(() => new Set());
  const preview: InlinePreview | undefined =
    picture.preview !== undefined && !failed.has(picture.preview.src) && arrived !== picture.src
      ? picture.preview
      : undefined;
  const src = preview?.src ?? picture.src;
  const width = preview?.width ?? picture.width;
  const height = preview?.height ?? picture.height;
  const size = width != null && height != null ? { width, height } : undefined;
  const known = size !== undefined;
  const waiting = arrived !== src;
  const guessed = !known && waiting;
  const arrive = () => {
    flushSync(() => {
      setArrived(src);
    });
  };
  useLayoutEffect(() => {
    keepStill();
  }, [arrived, failed, keepStill]);
  // A link's picture that cannot be loaded shows as the link it came from, not as a broken
  // picture; an attachment that fails is said to be unavailable.
  if (failed.has(picture.src)) {
    return picture.attachmentId === undefined ? (
      <a
        href={webPageUrl(picture.name)}
        target="_blank"
        rel="noreferrer"
        className="text-sm break-all text-accent underline-offset-2 hover:underline"
      >
        {picture.name}
      </a>
    ) : (
      <span className="text-sm text-ink-faint">{m.attachmentUnavailable}</span>
    );
  }
  return (
    <Button
      onPress={onOpen}
      aria-label={m.openImage}
      className="inline-block max-w-full rounded-md outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      <img
        src={src}
        alt={pictureAlt(picture, format(m.imageAlt, { name: picture.name }))}
        loading="lazy"
        // A finger dragging from the picture pans the list; it does not pick the picture up.
        draggable={false}
        referrerPolicy="no-referrer"
        {...(size ?? {})}
        style={size === undefined ? undefined : keptRoom(size.width, size.height)}
        onLoad={arrive}
        onError={() => {
          flushSync(() => {
            setFailed((before) => new Set(before).add(src));
          });
        }}
        className={
          imageClass +
          (known ? " h-auto" : "") +
          (guessed ? " h-48 w-48 object-cover" : "") +
          (waiting
            ? " animate-pulse bg-surface-hover motion-reduce:animate-none"
            : " bg-surface-sunken")
        }
      />
    </Button>
  );
}

/**
 * A video the server took a poster of: the poster, at its size, with a play control, which
 * swaps in a player of the original, so nothing of the video is fetched until the reader asks.
 * The player keeps the poster's room. A video the browser cannot play is offered for download
 * instead, as one without a poster always is.
 */
function InlineVideo({ attachment }: { attachment: Attachment }) {
  const m = useMessages();
  const keepStill = useKeepStill();
  const [playing, setPlaying] = useState(false);
  const [failed, setFailed] = useState(false);
  const poster = inlinePreview(attachment.preview);
  useLayoutEffect(() => {
    keepStill();
  }, [failed, keepStill]);
  if (poster === undefined || failed) {
    return <FileChip attachment={attachment} />;
  }
  const room = keptRoom(poster.width, poster.height);
  const label = attachment.description ?? attachment.fileName;
  return playing ? (
    <video
      src={attachment.downloadUrl}
      poster={poster.src}
      controls
      autoPlay
      playsInline
      aria-label={label}
      {...(attachment.description == null ? {} : { "aria-description": attachment.description })}
      width={poster.width}
      height={poster.height}
      style={room}
      onError={() => {
        flushSync(() => {
          setFailed(true);
        });
      }}
      className="block h-auto max-h-80 max-w-full rounded-md border border-line bg-black"
    />
  ) : (
    <Button
      onPress={() => {
        setPlaying(true);
      }}
      aria-label={format(m.playVideo, { title: label })}
      className="group relative inline-flex max-w-full items-center justify-center rounded-md outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      <img
        src={poster.src}
        alt=""
        loading="lazy"
        draggable={false}
        referrerPolicy="no-referrer"
        width={poster.width}
        height={poster.height}
        style={room}
        className={imageClass + " h-auto bg-surface-sunken"}
      />
      <span className="absolute flex h-14 w-14 items-center justify-center rounded-full bg-black/60 text-white shadow transition-transform group-hover:scale-110">
        <PlayIcon size={28} weight="fill" aria-hidden="true" />
      </span>
    </Button>
  );
}

/** A file to download, carrying its uploader's description, for a video they described. */
function FileChip({ attachment }: { attachment: Attachment }) {
  return (
    <a
      href={attachment.downloadUrl}
      {...(attachment.description == null ? {} : { "aria-description": attachment.description })}
      target="_blank"
      rel="noreferrer noopener"
      download={attachment.fileName}
      className="inline-flex items-center gap-1 rounded-md border border-line px-2 py-1 text-sm text-accent hover:bg-surface-hover"
    >
      <PaperclipIcon size={16} aria-hidden="true" />
      <span>{attachment.fileName}</span>
    </a>
  );
}

/** The control that takes one attachment off a message. */
function RemoveButton({ name, onPress }: { name: string; onPress: () => void }) {
  const m = useMessages();
  const label = format(m.removeSentAttachment, { name });
  return (
    <Tooltip text={label}>
      <Button
        aria-label={label}
        onPress={onPress}
        className="tap-target rounded-full border border-line bg-surface-raised p-1 text-ink-muted shadow-sm outline-none hover:text-danger focus-visible:ring-2 focus-visible:ring-accent/50"
      >
        <XIcon size={12} aria-hidden="true" />
      </Button>
    </Tooltip>
  );
}
