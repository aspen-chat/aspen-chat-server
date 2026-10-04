import type { LinkPreview } from "@aspen/protocol";
import { PlayIcon } from "@phosphor-icons/react";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A link to a video on an allowlisted provider: the thumbnail from our own store with a play
 * control, the title, and the provider. Playing swaps in the provider's player, sandboxed, so
 * no request reaches the provider before the reader asks. Called only with a preview whose
 * `video` passed `playerSrc` in `video.ts`; `player` is that checked URL.
 */
export function VideoCard({ preview, player }: { preview: LinkPreview; player: string }) {
  const m = useMessages();
  const [playing, setPlaying] = useState(false);
  const video = preview.video;
  if (video == null) {
    return null;
  }
  const title = preview.title ?? preview.url;
  const site = preview.siteName ?? new URL(preview.url).hostname;
  return (
    <div className="mt-1 w-full max-w-lg overflow-hidden rounded-md border border-line bg-surface-raised">
      <div
        className="relative w-full bg-surface-sunken"
        style={{ aspectRatio: `${String(video.width)} / ${String(video.height)}` }}
      >
        {playing ? (
          <iframe
            src={player}
            title={title}
            className="absolute inset-0 h-full w-full"
            sandbox="allow-scripts allow-same-origin allow-popups allow-popups-to-escape-sandbox allow-presentation"
            allow="autoplay; fullscreen; picture-in-picture; encrypted-media"
            referrerPolicy="strict-origin-when-cross-origin"
          />
        ) : (
          <Button
            onPress={() => {
              setPlaying(true);
            }}
            aria-label={format(m.playVideo, { title })}
            className="group absolute inset-0 flex h-full w-full items-center justify-center outline-none focus-visible:ring-2 focus-visible:ring-accent/60 focus-visible:ring-inset"
          >
            {preview.imageUrl != null && (
              <img
                src={preview.imageUrl}
                alt=""
                loading="lazy"
                className="absolute inset-0 h-full w-full object-cover"
              />
            )}
            <span className="relative flex h-14 w-14 items-center justify-center rounded-full bg-black/60 text-white shadow transition-transform group-hover:scale-110">
              <PlayIcon size={28} weight="fill" aria-hidden="true" />
            </span>
          </Button>
        )}
      </div>
      <div className="flex flex-col gap-0.5 px-3 py-2 text-sm">
        <a
          href={preview.url}
          target="_blank"
          rel="noreferrer noopener"
          className="font-medium wrap-anywhere text-accent hover:underline"
        >
          {title}
        </a>
        <span className="text-ink-muted">{site}</span>
      </div>
    </div>
  );
}
