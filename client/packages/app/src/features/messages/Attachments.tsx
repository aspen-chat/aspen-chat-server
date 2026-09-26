import type { Attachment } from "@aspen/protocol";
import { PaperclipIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useAttachments } from "@/api/hooks";
import { ImageGallery } from "@/features/messages/ImageGallery";
import { isImageType, splitInline, type Picture } from "@/features/messages/images";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const imageClass =
  "block max-h-80 max-w-full rounded-md border border-line object-contain bg-surface-sunken";

/**
 * What a message carries besides its text: uploaded attachments, shown inline when they are
 * images and as download chips otherwise, then links in the text that point straight at an
 * image, then links the server found to be images. Up to three pictures show inline; with
 * more, a button in their place opens the whole set in a gallery, and so does any picture.
 */
export function MessageMedia({
  attachmentIds,
  linkedImages,
  previewImages,
}: {
  attachmentIds: readonly string[];
  /** Links in the text that point straight at an image, from `imageUrls`. */
  linkedImages: readonly string[];
  /** Pictures the server found behind links, as `{ src, name }` with the link as the name. */
  previewImages: readonly Picture[];
}) {
  const m = useMessages();
  const attachments = useAttachments(attachmentIds);
  const [gallery, setGallery] = useState<number | null>(null);
  if (attachmentIds.length === 0 && linkedImages.length === 0 && previewImages.length === 0) {
    return null;
  }
  const pictures: Picture[] = [];
  const files: Attachment[] = [];
  const unavailable: string[] = [];
  attachments.forEach((attachment, i) => {
    if (attachment === undefined) {
      unavailable.push(attachmentIds[i] ?? "");
    } else if (isImageType(attachment.mimeType)) {
      pictures.push({ src: attachment.downloadUrl, name: attachment.fileName });
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
      {unavailable.map((id) => (
        <li key={id} className="text-sm text-ink-faint" data-attachment-id={id}>
          {m.attachmentUnavailable}
        </li>
      ))}
      {files.map((attachment) => (
        <li key={attachment.id} data-attachment-id={attachment.id}>
          <FileChip attachment={attachment} />
        </li>
      ))}
      {shown.map((picture, i) => (
        <li key={picture.src + String(i)}>
          <InlineImage
            picture={picture}
            onOpen={() => {
              setGallery(i);
            }}
          />
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

/** A picture in the message; pressing it opens the message's gallery on that picture. */
function InlineImage({ picture, onOpen }: { picture: Picture; onOpen: () => void }) {
  const m = useMessages();
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
        className={imageClass}
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
