import { plainFileName, type Attachment } from "@aspen/protocol";
import {
  ArrowClockwiseIcon,
  ArrowCounterClockwiseIcon,
  PauseIcon,
  PaperclipIcon,
  PlayIcon,
  SpeakerHighIcon,
  SpeakerLowIcon,
  SpeakerSlashIcon,
  XIcon,
} from "@phosphor-icons/react";
import { useLayoutEffect, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { Button, Slider, useLocale } from "react-aria-components";
import { useAttachments, useStore } from "@/api/hooks";
import { Tooltip } from "@/features/layout/Tooltip";
import { ImageGallery } from "@/features/messages/ImageGallery";
import {
  inlinePreview,
  isAudioType,
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
import { SliderRail } from "@/features/layout/SliderRail";
import { formatClock } from "@/features/layout/clock";
import { SpoilerCover } from "@/features/messages/SpoilerCard";
import { TextAttachment } from "@/features/messages/TextAttachment";
import { textLanguage } from "@/features/messages/textPreview";

const imageClass = "block max-h-80 max-w-full rounded-md border border-line object-contain";

/**
 * What a message carries besides its text: uploaded attachments, shown inline when they are
 * images, as their posters when they are videos the server took one of, in a small player when
 * they are sounds, as their first lines when they are text or code (`TextAttachment`), and as
 * download chips otherwise, then the pictures the server found behind links in the text, each
 * from its copy in storage. An attachment whose address is not one a deployment may serve
 * (`mediaUrl`) is unavailable. Up to three pictures show inline; with more, a button in their
 * place opens the whole set in a gallery, and so does any picture. Inline, a picture shows the
 * smaller copy the server made of it where there is one; the gallery shows the original. With
 * `onRemove`, each attachment shown carries a control that takes it off the message. An
 * attachment its uploader marked a spoiler is covered (`SpoilerCover`), whatever it is, until the
 * reader reveals it, its own controls left usable over the cover.
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
  // The spoilers the reader has revealed here, by attachment, which stay revealed while the
  // message is shown, in the gallery too.
  const [revealed, setRevealed] = useState<ReadonlySet<string>>(() => new Set());
  if (attachmentIds.length === 0 && previewImages.length === 0) {
    return null;
  }
  const covered = (id: string | undefined, spoiler: boolean | undefined) =>
    spoiler === true && id !== undefined && !revealed.has(id);
  const reveal = (id: string) => {
    setRevealed((before) => new Set(before).add(id));
  };
  const pictures: Picture[] = [];
  const files: Attachment[] = [];
  const texts: { attachment: Attachment; language: string | null }[] = [];
  const videos: Attachment[] = [];
  const sounds: Attachment[] = [];
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
        spoiler: attachment.spoiler === true,
      });
    } else if (isVideoType(attachment.mimeType) && attachment.preview != null) {
      videos.push(attachment);
    } else if (isAudioType(attachment.mimeType)) {
      sounds.push(attachment);
    } else {
      const language = textLanguage(attachment.fileName, attachment.mimeType);
      if (language === undefined) {
        files.push(attachment);
      } else {
        texts.push({ attachment, language });
      }
    }
  });
  pictures.push(...previewImages);
  const { shown, hidden } = splitInline(pictures);
  // An attachment's own controls, which stay usable over its spoiler's cover; `overlay` draws
  // them over a picture or card rather than beside a chip.
  const controls = (id: string, name: string, overlay: boolean) => (
    <>
      {onRemove !== undefined && (
        <RemoveButton
          name={name}
          onPress={() => {
            onRemove(id);
          }}
        />
      )}
      <CopyIdButton
        id={id}
        thing="attachment"
        {...(overlay ? { className: "bg-surface-raised/90 shadow-sm" } : {})}
      />
    </>
  );
  const chip = (attachment: Attachment) =>
    covered(attachment.id, attachment.spoiler) ? (
      <SpoilerCover
        compact
        onReveal={() => {
          reveal(attachment.id);
        }}
      >
        <FileChip attachment={attachment} />
      </SpoilerCover>
    ) : (
      <FileChip attachment={attachment} />
    );
  const sound = (attachment: Attachment) =>
    covered(attachment.id, attachment.spoiler) ? (
      <SpoilerCover
        onReveal={() => {
          reveal(attachment.id);
        }}
        className="max-w-full"
      >
        <InlineAudio attachment={attachment} />
      </SpoilerCover>
    ) : (
      <InlineAudio attachment={attachment} />
    );
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
      {[...files, ...sounds].map((attachment) => (
        <li
          key={attachment.id}
          data-attachment-id={attachment.id}
          className="flex min-w-0 max-w-full items-center gap-1"
        >
          {isAudioType(attachment.mimeType) ? sound(attachment) : chip(attachment)}
          {controls(attachment.id, attachment.fileName, false)}
        </li>
      ))}
      {texts.map(({ attachment, language }) => (
        <li
          key={attachment.id}
          data-attachment-id={attachment.id}
          className="relative w-full min-w-0 max-w-3xl"
        >
          <TextAttachment
            attachment={attachment}
            language={language}
            chip={chip(attachment)}
            controls={controls(attachment.id, attachment.fileName, false)}
            {...(covered(attachment.id, attachment.spoiler)
              ? {
                  onReveal: () => {
                    reveal(attachment.id);
                  },
                }
              : {})}
          />
        </li>
      ))}
      {videos.map((attachment) => (
        <li
          key={attachment.id}
          data-attachment-id={attachment.id}
          className="relative min-w-0 max-w-full"
        >
          {covered(attachment.id, attachment.spoiler) ? (
            <SpoilerCover
              onReveal={() => {
                reveal(attachment.id);
              }}
              className="max-w-full"
            >
              <InlineVideo attachment={attachment} />
            </SpoilerCover>
          ) : (
            <InlineVideo attachment={attachment} />
          )}
          <span className="absolute top-1 end-1 flex gap-1">
            {controls(attachment.id, attachment.fileName, true)}
          </span>
        </li>
      ))}
      {shown.map((picture, i) => (
        <li key={picture.src + String(i)} className="relative min-w-0 max-w-full">
          {covered(picture.attachmentId, picture.spoiler) ? (
            <SpoilerCover
              onReveal={() => {
                if (picture.attachmentId !== undefined) {
                  reveal(picture.attachmentId);
                }
              }}
              className="max-w-full"
            >
              <InlineImage picture={picture} onOpen={() => undefined} />
            </SpoilerCover>
          ) : (
            <InlineImage
              picture={picture}
              onOpen={() => {
                setGallery(i);
              }}
            />
          )}
          {picture.attachmentId !== undefined && (
            <span className="absolute top-1 end-1 flex gap-1">
              {controls(picture.attachmentId, picture.name, true)}
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
            aria-label={format(m.viewAllImages, { count: pictures.length })}
            className="flex h-32 w-32 items-center justify-center rounded-md border border-line bg-surface-sunken text-lg font-semibold text-ink-muted outline-none hover:bg-surface-hover pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            {format(m.moreImages, { count: hidden })}
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
          isCovered={(picture) => covered(picture.attachmentId, picture.spoiler)}
          onReveal={reveal}
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
    // Named by the picture's own text alternative, its description where it has one: a label on
    // the button would hide it, since a button's content is presentational to assistive technology.
    <Button
      onPress={onOpen}
      data-media-url={picture.src}
      data-media-name={picture.name}
      data-media-picture
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
      data-media-url={attachment.downloadUrl}
      data-media-name={attachment.fileName}
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

/**
 * How many steps a sound's position slider counts from its start to its end: one press of an
 * arrow key moves it a step, and the thumb comes to rest on one as it is dragged. A step is about a
 * second, or a tenth of one in a sound too short for whole seconds to scrub through, and divides
 * the length exactly. The slider counts steps rather than seconds because React Aria snaps a value
 * to a whole number of steps from the minimum, and would hold the thumb short of a length that is
 * not one.
 */
function scrubSteps(duration: number): number {
  return Math.max(1, Math.round(duration / (duration < 10 ? 0.1 : 1)));
}

/**
 * The volume the reader last chose in any sound's player, which each player starts at, so a
 * reader who turned one down need not turn down the next. It lasts as long as the page.
 */
let chosenVolume = 1;

let canSetVolume: boolean | undefined;

/**
 * Whether the page may set a sound's volume. iOS keeps every element at full volume, leaving it
 * to the device's buttons, and reads back 1 whatever was set; muting still works there.
 */
function volumeSettable(): boolean {
  if (canSetVolume === undefined) {
    const probe = new Audio();
    probe.volume = 0.5;
    canSetVolume = probe.volume === 0.5;
  }
  return canSetVolume;
}

/**
 * A sound in a small player: its name, which downloads it, a mute control and a volume, a play
 * and pause control (a replay control once it has played to its end), a position to scrub, and
 * the time played against its length. Only the sound's length and the start of its file are
 * fetched until it is played. The player is as wide as its whole name needs, from a floor up to
 * the width it has, and otherwise one size whatever it plays, so its room is kept from the first.
 * A sound the browser cannot play is offered for download instead, and one whose length the file
 * does not give (a recording streamed as it was made) plays without a position to scrub until the
 * browser has found its end.
 */
function InlineAudio({ attachment }: { attachment: Attachment }) {
  const m = useMessages();
  const { direction } = useLocale();
  const keepStill = useKeepStill();
  const audio = useRef<HTMLAudioElement>(null);
  const position = useRef<HTMLInputElement>(null);
  const [failed, setFailed] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [ended, setEnded] = useState(false);
  const [time, setTime] = useState(0);
  const [duration, setDuration] = useState<number | null>(null);
  // Where the thumb is while it is dragged; the sound moves there when it is let go.
  const [scrubbing, setScrubbing] = useState<number | null>(null);
  const [volume, setVolume] = useState(chosenVolume);
  const [muted, setMuted] = useState(false);
  const silent = muted || volume === 0;
  const shown = scrubbing ?? time;
  const elapsed = formatClock(shown, duration ?? shown);
  const total = duration === null ? null : formatClock(duration);
  const valueText = total === null ? elapsed : format(m.audio.position, { elapsed, total });
  const steps = duration === null || duration === 0 ? 0 : scrubSteps(duration);
  const toSeconds = (step: number) => (steps === 0 ? 0 : (step / steps) * (duration ?? 0));
  useLayoutEffect(() => {
    keepStill();
  }, [failed, keepStill]);
  // React Aria words a slider's value as a number, which would read out seconds; as in
  // `StepSlider`, the input's value text is set once React Aria has written its own.
  useLayoutEffect(() => {
    position.current?.setAttribute("aria-valuetext", valueText);
  });
  useLayoutEffect(() => {
    const element = audio.current;
    if (element !== null) {
      element.volume = volume;
      element.muted = muted;
    }
  }, [volume, muted, failed]);
  if (failed) {
    return <FileChip attachment={attachment} />;
  }
  const label = attachment.description ?? attachment.fileName;
  const learnDuration = () => {
    const length = audio.current?.duration;
    setDuration(length !== undefined && Number.isFinite(length) ? length : null);
  };
  return (
    <div
      role="group"
      aria-label={label}
      {...(attachment.description == null ? {} : { "aria-description": attachment.description })}
      data-media-url={attachment.downloadUrl}
      data-media-name={attachment.fileName}
      className="flex w-max min-w-[min(24rem,100%)] max-w-full flex-col gap-2 rounded-md border border-line bg-surface-sunken px-4 py-3"
    >
      <audio
        ref={audio}
        src={attachment.downloadUrl}
        preload="metadata"
        onLoadedMetadata={learnDuration}
        onDurationChange={learnDuration}
        onTimeUpdate={(event) => {
          setTime(event.currentTarget.currentTime);
        }}
        onPlay={() => {
          setPlaying(true);
          setEnded(false);
        }}
        onSeeked={(event) => {
          setEnded(event.currentTarget.ended);
        }}
        onEnded={(event) => {
          // The last `timeupdate` may come short of the end, and a length estimated from the
          // file's start may be revised, so the position is put at the end it reached.
          const element = event.currentTarget;
          const length = Number.isFinite(element.duration) ? element.duration : element.currentTime;
          setDuration(length);
          setTime(length);
          setEnded(true);
        }}
        onPause={() => {
          setPlaying(false);
        }}
        onError={() => {
          flushSync(() => {
            setFailed(true);
          });
        }}
      />
      <div className="flex items-center gap-1">
        <a
          href={attachment.downloadUrl}
          target="_blank"
          rel="noreferrer noopener"
          download={attachment.fileName}
          className="flex min-w-0 flex-1 items-center gap-1 text-sm text-accent hover:underline"
        >
          <PaperclipIcon size={14} className="shrink-0" aria-hidden="true" />
          <span className="truncate">{attachment.fileName}</span>
        </a>
        <Tooltip text={silent ? m.audio.unmute : m.audio.mute}>
          <Button
            aria-label={format(silent ? m.audio.unmuteTitle : m.audio.muteTitle, { title: label })}
            onPress={() => {
              if (silent) {
                setMuted(false);
                // Unmuting a sound turned all the way down brings it back to be heard.
                if (volume === 0) {
                  setVolume(1);
                  chosenVolume = 1;
                }
              } else {
                setMuted(true);
              }
            }}
            className="tap-target shrink-0 rounded-md p-1 text-ink-muted outline-none hover:bg-surface-hover hover:text-ink focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            {silent ? (
              <SpeakerSlashIcon size={16} aria-hidden="true" />
            ) : volume < 0.5 ? (
              <SpeakerLowIcon size={16} aria-hidden="true" />
            ) : (
              <SpeakerHighIcon size={16} aria-hidden="true" />
            )}
          </Button>
        </Tooltip>
        {volumeSettable() && (
          <Slider
            aria-label={format(m.audio.volume, { title: label })}
            value={muted ? 0 : volume}
            minValue={0}
            maxValue={1}
            step={0.05}
            formatOptions={{ style: "percent" }}
            onChange={(value) => {
              if (typeof value === "number") {
                setVolume(value);
                setMuted(false);
                chosenVolume = value;
              }
            }}
            className="w-20 shrink-0"
          >
            <SliderRail filled />
          </Slider>
        )}
      </div>
      <div className="flex items-center gap-2">
        <Button
          aria-label={format(playing ? m.audio.pause : ended ? m.audio.replay : m.audio.play, {
            title: label,
          })}
          onPress={() => {
            const element = audio.current;
            if (element === null) {
              return;
            }
            if (element.paused) {
              // A refusal (no interaction with the page yet, the file gone) leaves it paused,
              // as its control shows; an unplayable file is answered by `onError`.
              element.play().catch(() => undefined);
            } else {
              element.pause();
            }
          }}
          className="tap-target flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-accent text-accent-contrast outline-none hover:bg-accent-strong pressed:opacity-80 focus-visible:ring-2 focus-visible:ring-accent/50"
        >
          {playing ? (
            <PauseIcon size={16} weight="fill" aria-hidden="true" />
          ) : ended ? (
            // The arrow turns back against the direction of reading.
            direction === "rtl" ? (
              <ArrowClockwiseIcon size={16} weight="bold" aria-hidden="true" />
            ) : (
              <ArrowCounterClockwiseIcon size={16} weight="bold" aria-hidden="true" />
            )
          ) : (
            <PlayIcon size={16} weight="fill" aria-hidden="true" />
          )}
        </Button>
        <Slider
          aria-label={format(m.audio.seek, { title: label })}
          value={steps === 0 ? 0 : (Math.min(shown, duration ?? 0) / (duration ?? 1)) * steps}
          minValue={0}
          maxValue={steps}
          isDisabled={steps === 0}
          onChange={(value) => {
            if (typeof value === "number") {
              setScrubbing(toSeconds(value));
            }
          }}
          onChangeEnd={(value) => {
            if (typeof value === "number") {
              const seconds = toSeconds(value);
              if (audio.current !== null) {
                audio.current.currentTime = seconds;
              }
              setTime(seconds);
              setScrubbing(null);
            }
          }}
          // The thumb's centre travels to the track's ends, so half of it overhangs each; the
          // margin keeps that half clear of the play control and the time.
          className="mx-2 min-w-0 flex-1 disabled:opacity-60"
        >
          <SliderRail inputRef={position} filled />
        </Slider>
        <span aria-hidden="true" className="shrink-0 text-xs tabular-nums text-ink-muted">
          {total === null ? elapsed : `${elapsed} / ${total}`}
        </span>
      </div>
    </div>
  );
}

/** A file to download, carrying its uploader's description, for a video they described. */
function FileChip({ attachment }: { attachment: Attachment }) {
  return (
    <a
      href={attachment.downloadUrl}
      data-media-url={attachment.downloadUrl}
      data-media-name={attachment.fileName}
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
