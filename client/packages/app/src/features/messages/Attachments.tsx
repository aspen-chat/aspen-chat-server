import type { Attachment } from "@aspen/protocol";
import { PaperclipIcon, XIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useAttachments, useStore } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { ImageGallery } from "@/features/messages/ImageGallery";
import { isImageType, splitInline, type Picture } from "@/features/messages/images";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { CopyIdButton } from "@/features/layout/CopyId";

const imageClass = "block max-h-80 max-w-full rounded-md border border-line object-contain";

/**
 * What a message carries besides its text: uploaded attachments, shown inline when they are
 * images and as download chips otherwise, then links in the text that point straight at an
 * image, then links the server found to be images. Up to three pictures show inline; with
 * more, a button in their place opens the whole set in a gallery, and so does any picture.
 * With `onRemove`, each attachment shown carries a control that takes it off the message.
 */
export function MessageMedia({
  attachmentIds,
  linkedImages,
  previewImages,
  onRemove,
}: {
  attachmentIds: readonly string[];
  /** Links in the text that point straight at an image, from `imageUrls`. */
  linkedImages: readonly string[];
  /** Pictures the server found behind links, as `{ src, name }` with the link as the name. */
  previewImages: readonly Picture[];
  /** Takes an attachment off the message, for its author and those who manage messages. */
  onRemove?: (attachmentId: string) => void;
}) {
  const m = useMessages();
  const attachments = useAttachments(attachmentIds);
  const store = useStore();
  const [gallery, setGallery] = useState<number | null>(null);
  if (attachmentIds.length === 0 && linkedImages.length === 0 && previewImages.length === 0) {
    return null;
  }
  const pictures: Picture[] = [];
  const files: Attachment[] = [];
  const unavailable: string[] = [];
  const coming: string[] = [];
  attachments.forEach((attachment, i) => {
    const id = attachmentIds[i] ?? "";
    if (attachment === undefined) {
      (store.missing("attachment", id) ? unavailable : coming).push(id);
    } else if (isImageType(attachment.mimeType)) {
      pictures.push({
        src: attachment.downloadUrl,
        name: attachment.fileName,
        attachmentId: attachment.id,
        width: attachment.width,
        height: attachment.height,
      });
    } else {
      files.push(attachment);
    }
  });
  for (const url of linkedImages) {
    pictures.push({ src: url, name: url });
  }
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
      {shown.map((picture, i) => (
        <li key={picture.src + String(i)} className="relative">
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
      <ImageGallery
        pictures={pictures}
        initial={gallery ?? 0}
        isOpen={gallery !== null}
        onClose={() => {
          setGallery(null);
        }}
      />
    </ul>
  );
}

/**
 * A picture in the message; pressing it opens the message's gallery on that picture. A picture
 * whose size is known keeps exactly its room while it loads; one whose size is not keeps a
 * conservative guess at it, which the channel's view holds still through when the picture
 * comes in larger or smaller. Either pulses as a skeleton until it has loaded.
 */
function InlineImage({ picture, onOpen }: { picture: Picture; onOpen: () => void }) {
  const m = useMessages();
  const [loaded, setLoaded] = useState<string | null>(null);
  const known = picture.width != null && picture.height != null;
  const waiting = loaded !== picture.src;
  return (
    <Button
      onPress={onOpen}
      aria-label={m.openImage}
      className="inline-block rounded-md outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
    >
      <img
        src={picture.src}
        alt={format(m.imageAlt, { name: picture.name })}
        loading="lazy"
        referrerPolicy="no-referrer"
        {...(known ? { width: picture.width ?? 0, height: picture.height ?? 0 } : {})}
        onLoad={() => {
          setLoaded(picture.src);
        }}
        onError={() => {
          setLoaded(picture.src);
        }}
        className={
          imageClass +
          (known ? " h-auto w-auto" : "") +
          (waiting
            ? " animate-pulse bg-surface-hover motion-reduce:animate-none" +
              (known ? "" : " min-h-48 min-w-48")
            : " bg-surface-sunken")
        }
      />
    </Button>
  );
}

function FileChip({ attachment }: { attachment: Attachment }) {
  return (
    <a
      href={attachment.downloadUrl}
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
