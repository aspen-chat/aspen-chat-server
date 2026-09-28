import { linkify } from "@/features/messages/linkify";

/** A picture a message shows: where its bytes are and what to call it. */
export interface Picture {
  src: string;
  name: string;
  /** The attachment it is, when it is one rather than a link. */
  attachmentId?: string;
}

/** How many of a message's pictures show inline; the rest are behind the gallery button. */
export const INLINE_IMAGE_LIMIT = 3;

/** The pictures to show inline and how many the gallery button must account for. */
export function splitInline(pictures: readonly Picture[]): {
  shown: readonly Picture[];
  hidden: number;
} {
  if (pictures.length <= INLINE_IMAGE_LIMIT) {
    return { shown: pictures, hidden: 0 };
  }
  return {
    shown: pictures.slice(0, INLINE_IMAGE_LIMIT),
    hidden: pictures.length - INLINE_IMAGE_LIMIT,
  };
}

/** File extensions a browser renders as an image in an `<img>`. */
const IMAGE_EXTENSIONS = /\.(png|jpe?g|gif|webp|avif|bmp|svg)$/i;

/** Whether a MIME type is one to show inline as a picture. */
export function isImageType(mimeType: string): boolean {
  return mimeType.toLowerCase().startsWith("image/");
}

/** Whether a URL's path ends in an image extension, ignoring any query or fragment. */
export function isImageUrl(url: string): boolean {
  try {
    return IMAGE_EXTENSIONS.test(new URL(url).pathname);
  } catch {
    return false;
  }
}

/** Every distinct link in a message that points straight at an image, in order of appearance. */
export function imageUrls(content: string): string[] {
  const urls: string[] = [];
  for (const run of linkify(content)) {
    if (run.kind === "link" && isImageUrl(run.url) && !urls.includes(run.url)) {
      urls.push(run.url);
    }
  }
  return urls;
}

/**
 * Whether a message is nothing but links to pictures separated by whitespace, in which case
 * only the pictures are shown. `isPicture` says whether a given link resolves to an image,
 * by its extension or by a server preview that turned out to be an image.
 */
export function onlyImageLinks(content: string, isPicture: (url: string) => boolean): boolean {
  const runs = linkify(content.trim());
  const links = runs.filter((run) => run.kind === "link");
  return (
    links.length > 0 &&
    runs.every((run) => run.kind === "link" || run.text.trim().length === 0) &&
    links.every((run) => isPicture(run.url))
  );
}
