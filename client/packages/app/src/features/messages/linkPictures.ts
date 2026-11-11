import type { LinkPreview } from "@aspen/protocol";
import { imageUrls, onlyImageLinks } from "@/features/messages/images";
import { normalizedUrl, spoileredLinks } from "@/features/messages/spoileredLinks";

/** A picture the server found behind a link, named by the link. */
export interface LinkPicture {
  readonly src: string;
  readonly name: string;
  readonly width: number | null | undefined;
  readonly height: number | null | undefined;
}

/** What a message's link previews are drawn as. */
export interface LinkPictures {
  /** The previews drawn as pictures among the message's own. */
  readonly pictures: readonly LinkPicture[];
  /** The previews drawn as cards. */
  readonly cards: readonly LinkPreview[];
  /** Whether a preview is of a link inside a spoiler, and so drawn covered. */
  readonly isSpoilered: (preview: LinkPreview) => boolean;
  /** Whether the message is nothing but links drawn as pictures, so its text is left out. */
  readonly pictureOnly: boolean;
}

/**
 * The form two addresses are compared in: a link as the message's text writes it against a
 * preview's `url`, which the server writes as a WHATWG URL serializes it (the host in lower
 * case, a path's other scripts percent-encoded, `/` for no path). The text itself where it is
 * no URL.
 */
const comparable = (url: string) => normalizedUrl(url) ?? url;

/** A preview the server made from a link that is itself an image: a picture, no text. */
function isPictureOnly(preview: LinkPreview): boolean {
  return preview.imageUrl != null && preview.title == null && preview.description == null;
}

/**
 * Sorts `previews`, the link previews of a message whose text is `content`, into pictures and
 * cards. A preview's `imageUrl` is taken as given: the caller leaves out one that may not be
 * shown.
 */
export function linkPictures(content: string, previews: readonly LinkPreview[]): LinkPictures {
  // A preview of a link inside a spoiler would give the spoiler away, so it is always a card,
  // never a picture among the message's, and drawn covered (`SpoilerCard`).
  const spoilered = previews.length === 0 ? null : spoileredLinks(content);
  const isSpoilered = (p: LinkPreview) => spoilered?.has(comparable(p.url)) === true;
  // A link that looks like a picture by its extension is drawn from the preview's picture: a
  // share page named like a gif (tenor.com/….gif) is a page, and only the preview's picture,
  // its og:image, is the gif. Until the preview comes, or when it has no picture, the link
  // stays a link in the text.
  const found = new Map(
    previews.flatMap((p) =>
      p.imageUrl == null || isSpoilered(p) ? [] : [[comparable(p.url), p] as const],
    ),
  );
  // Two links that differ only as written are one address, and one picture.
  const shown = new Set<string>();
  const foundImages = imageUrls(content).flatMap((url) => {
    const address = comparable(url);
    const p = found.get(address);
    if (p?.imageUrl == null || shown.has(address)) {
      return [];
    }
    shown.add(address);
    return [{ src: p.imageUrl, name: url, width: p.imageWidth, height: p.imageHeight }];
  });
  // A link already shown as a picture needs no card for the same URL.
  const rest = previews.filter((p) => !shown.has(comparable(p.url)));
  // Links the server found to be images themselves join the message's pictures; the rest
  // are cards.
  const pictures = [
    ...foundImages,
    ...rest.flatMap((p) =>
      isPictureOnly(p) && p.imageUrl != null && !isSpoilered(p)
        ? [{ src: p.imageUrl, name: p.url, width: p.imageWidth, height: p.imageHeight }]
        : [],
    ),
  ];
  const drawn = new Set(pictures.map((picture) => comparable(picture.name)));
  return {
    pictures,
    cards: rest.filter((p) => !isPictureOnly(p) || isSpoilered(p)),
    isSpoilered,
    pictureOnly: onlyImageLinks(content, (url) => drawn.has(comparable(url))),
  };
}
