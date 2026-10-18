/**
 * What was right-clicked inside a message, which its context menu offers first: a picture,
 * video, or file attached to it, or a link's picture (`data-media-url`, with its file name in
 * `data-media-name`, and `data-media-picture` on a picture), or a link in its text that opens
 * elsewhere.
 */
export type UnderPointer =
  | {
      readonly kind: "media";
      readonly url: string;
      readonly name: string;
      readonly picture: boolean;
    }
  | { readonly kind: "link"; readonly url: string; readonly anchor: HTMLAnchorElement }
  | null;

/** What of a message `target`, right-clicked inside it, is. */
export function underPointer(target: EventTarget | null): UnderPointer {
  if (!(target instanceof Element)) {
    return null;
  }
  const media = target.closest<HTMLElement>("[data-media-url]");
  if (media?.dataset.mediaUrl !== undefined) {
    return {
      kind: "media",
      url: media.dataset.mediaUrl,
      name: media.dataset.mediaName ?? "",
      picture: media.dataset.mediaPicture !== undefined,
    };
  }
  // Links within the app (invites, links to messages) offer their own menus or go nowhere new.
  const anchor = target.closest<HTMLAnchorElement>("a[href][target='_blank']");
  return anchor === null ? null : { kind: "link", url: anchor.href, anchor };
}
