import type { Attachment } from "@aspen/protocol";
import { useEffect, useLayoutEffect, useState, type ReactNode } from "react";
import { Button } from "react-aria-components";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { CodeBlock } from "@/features/messages/CodeBlock";
import { useKeepStill } from "@/features/messages/keepStill";
import { saveFrom, toastFileFailure } from "@/features/messages/mediaFile";
import { SpoilerCover } from "@/features/messages/SpoilerCard";
import { keptHead, textHead, type TextHead } from "@/features/messages/textPreview";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * A text or code file's first `TEXT_PREVIEW_LINES` lines, under its download `chip` and its
 * `controls`, drawn as a code block is: highlighted as `language`, or as written when that is
 * `null`. A file with more than that ends in a Download to see more button, which saves the
 * whole file. Until its lines arrive a few lines' worth of skeleton stands in, a conservative
 * guess, and the list keeps the view still when they replace it (`useKeepStill`). A file that
 * cannot be read, or turns out not to be text, shows its chip and controls alone. With
 * `onReveal` it is a spoiler not yet revealed: the card is covered, its lines read all the same
 * so that revealing them moves nothing, and its controls lie over the cover. The chip is the
 * caller's, covered on its own where it stands alone.
 */
export function TextAttachment({
  attachment,
  language,
  chip,
  controls,
  onReveal,
}: {
  attachment: Attachment;
  language: string | null;
  chip: ReactNode;
  controls: ReactNode;
  onReveal?: () => void;
}) {
  const m = useMessages();
  const keepStill = useKeepStill();
  const url = attachment.downloadUrl;
  const [head, setHead] = useState<TextHead | null>(() => keptHead(url) ?? null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (head !== null || failed) {
      return;
    }
    let cancelled = false;
    textHead(url, attachment.mimeType).then(
      (read) => {
        if (!cancelled) {
          setHead(read);
        }
      },
      () => {
        if (!cancelled) {
          setFailed(true);
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [url, attachment.mimeType, head, failed]);
  useLayoutEffect(() => {
    keepStill();
  }, [head, failed, keepStill]);
  if (failed) {
    return (
      <div className="flex items-center gap-1">
        {chip}
        {controls}
      </div>
    );
  }
  const covered = onReveal !== undefined;
  const card = (
    <div
      data-media-url={url}
      data-media-name={attachment.fileName}
      {...(head === null ? { "aria-busy": true } : {})}
      className="text-attachment w-full overflow-hidden rounded-md border border-line bg-surface-sunken"
    >
      <div className="flex items-center gap-1 border-b border-line p-1">
        {covered ? <FileName name={attachment.fileName} /> : chip}
        {!covered && controls}
      </div>
      {head === null ? (
        <>
          <LoadingLabel />
          <Skeleton className="m-2 h-24 rounded" />
        </>
      ) : (
        <CodeBlock
          code={head.text}
          language={language}
          label={format(m.textAttachmentLabel, { name: attachment.fileName })}
        />
      )}
      {head?.more === true && (
        <div className="flex justify-center border-t border-line p-1.5">
          <Button
            onPress={() => {
              saveFrom(url, attachment.fileName).catch((failure: unknown) => {
                toastFileFailure(m, failure);
              });
            }}
            className="rounded-md px-3 py-1 text-sm font-medium text-accent outline-none hover:bg-surface-hover pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            {m.downloadToSeeMore}
          </Button>
        </div>
      )}
    </div>
  );
  return covered ? (
    <>
      <SpoilerCover onReveal={onReveal} className="w-full">
        {card}
      </SpoilerCover>
      <span className="absolute top-1 end-1 flex gap-1">{controls}</span>
    </>
  ) : (
    card
  );
}

/** A covered card's name, drawn as its chip is, to be blurred with the rest. */
function FileName({ name }: { name: string }) {
  return <span className="rounded-md border border-line px-2 py-1 text-sm">{name}</span>;
}
