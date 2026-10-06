import type { CSSProperties } from "react";
import { linkify } from "@/features/messages/linkify";

/** A picture a message shows: where its bytes are and what to call it. */
export interface Picture {
  src: string;
  name: string;
  /** The attachment it is, when it is one rather than a link. */
  attachmentId?: string;
  /** Its size in pixels, when known before it loads, so its room is kept for it. */
  width?: number | null | undefined;
  height?: number | null | undefined;
  /** What it shows, in its uploader's words, when it is an attachment they described. */
  description?: string | null | undefined;
  /**
   * The smaller copy the server made for showing it inline, when it made one worth having; the
   * gallery shows `src`, the original.
   */
  preview?: InlinePreview | undefined;
}

/** A smaller copy of a picture, for showing it inline, and its size in pixels. */
export interface InlinePreview {
  src: string;
  width: number;
  height: number;
}

/** The inline copy an attachment's record names, when it has one. */
export function inlinePreview(
  preview: { url: string; width: number; height: number } | null | undefined,
): InlinePreview | undefined {
  return preview == null
    ? undefined
    : { src: preview.url, width: preview.width, height: preview.height };
}

/** Whether a MIME type is a video's. */
export function isVideoType(mimeType: string): boolean {
  return mimeType.toLowerCase().startsWith("video/");
}

/** Whether a file is one a description is offered for: a picture or a video. */
export function describable(mimeType: string): boolean {
  return mimeType.startsWith("image/") || mimeType.startsWith("video/");
}

/**
 * The text that stands for a picture where it cannot be seen: its uploader's description, or
 * else `unnamed` (the name it goes by, worded as a picture's).
 */
export function pictureAlt(picture: Picture, unnamed: string): string {
  return picture.description ?? unnamed;
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

/**
 * The room a picture of known size takes before and after it loads: as wide as it is, or as
 * the 320px it may be tall (`max-h-80`) allows, and no wider than its column (`max-w-full`),
 * at its own proportions. Width and proportions are set outright, because an image's room is
 * otherwise worked out from the image itself, which is nothing until it arrives.
 */
export function keptRoom(width: number, height: number): CSSProperties {
  return {
    // Wider than its column, `max-w-full` narrows it, and its proportions keep its height.
    width: `${String(Math.min(width, (MAX_PICTURE_HEIGHT * width) / height))}px`,
    aspectRatio: `${String(width)} / ${String(height)}`,
  };
}

/** The tallest a picture is drawn inline, as `max-h-80`. */
const MAX_PICTURE_HEIGHT = 320;
