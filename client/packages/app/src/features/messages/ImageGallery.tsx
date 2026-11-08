import { ArrowSquareOutIcon, CaretLeftIcon, CaretRightIcon, XIcon } from "@phosphor-icons/react";
import {
  useEffect,
  useRef,
  useState,
  type MouseEvent,
  type PointerEvent,
  type TransitionEvent,
} from "react";
import { Button, Dialog, Link, Modal, ModalOverlay, useLocale } from "react-aria-components";
import { overlayLayerClass } from "@/features/invites/dialog";
import { useMotion } from "@/features/layout/motion";
import { TOUCH_ONLY, TOUCH_SCREEN, useMediaQuery } from "@/features/layout/useMediaQuery";
import { useSwipe } from "@/features/layout/useSwipe";
import { swipeOutcome } from "@/features/messages/gallerySwipe";
import { pictureAlt, type Picture } from "@/features/messages/images";
import { rubberBand } from "@/features/messages/scrollPhysics";
import { SpoilerCover } from "@/features/messages/SpoilerCard";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const navButtonClass =
  "rounded-full bg-black/50 p-2 text-white outline-none hover:bg-black/70 pressed:bg-black/80 " +
  "disabled:opacity-30 focus-visible:ring-2 focus-visible:ring-white/70";

/**
 * Where the pictures are under a finger: at rest on the current one; following a finger,
 * `travel` pixels along the inline axis from their places; drawn `travel` from their places,
 * about to ease into them; or easing into them.
 */
type Slide =
  | { readonly kind: "still" }
  | { readonly kind: "dragging"; readonly travel: number }
  | { readonly kind: "placed"; readonly travel: number }
  | { readonly kind: "easing" };

/**
 * Every picture of a message, one at a time, over a darkened page, with its uploader's
 * description beneath it when it has one: arrows and the arrow keys move between them, a strip
 * of thumbnails jumps to one, and Escape, the close control, or a click on the empty space
 * around the picture leaves. On a touch screen (`TOUCH_SCREEN`) a finger swipes between them:
 * the pictures follow it, its neighbours drawn beside the current one, and when it lifts they
 * go the way it was thrown, or, if it had come to rest, to whichever picture is more than
 * halfway in; pulled past the first or last picture they give a little and spring back. On a
 * touch-only device (`TOUCH_ONLY`) the arrows are left out, since the picture needs the width
 * they take on a phone. Opens on the picture at `initial`. A spoiler the reader has not
 * revealed (`isCovered`) is covered here as in the message, its thumbnail blurred and its
 * description left out, until they reveal it (`onReveal`, with its attachment's id).
 */
export function ImageGallery({
  pictures,
  initial,
  isOpen,
  onClose,
  isCovered,
  onReveal,
}: {
  pictures: readonly Picture[];
  initial: number;
  isOpen: boolean;
  onClose: () => void;
  isCovered: (picture: Picture) => boolean;
  onReveal: (attachmentId: string) => void;
}) {
  const m = useMessages();
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
      isDismissable
      className={
        "fixed inset-x-0 top-(--title-bar-height) bottom-0 flex items-center justify-center bg-black/80 motion-backdrop " +
        overlayLayerClass
      }
    >
      <Modal className="h-full w-full outline-none motion-dialog">
        <Dialog aria-label={m.gallery.label} className="flex h-full w-full flex-col outline-none">
          {({ close }) => (
            <GalleryBody
              pictures={pictures}
              initial={initial}
              close={close}
              isCovered={isCovered}
              onReveal={onReveal}
            />
          )}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function GalleryBody({
  pictures,
  initial,
  close,
  isCovered,
  onReveal,
}: {
  pictures: readonly Picture[];
  initial: number;
  close: () => void;
  isCovered: (picture: Picture) => boolean;
  onReveal: (attachmentId: string) => void;
}) {
  const m = useMessages();
  const { direction } = useLocale();
  const touchOnly = useMediaQuery(TOUCH_ONLY);
  const touchScreen = useMediaQuery(TOUCH_SCREEN);
  const motion = useMotion();
  const [index, setIndex] = useState(Math.min(initial, pictures.length - 1));
  const current = pictures[index];
  const previous = () => {
    setIndex((i) => Math.max(0, i - 1));
  };
  const next = () => {
    setIndex((i) => Math.min(pictures.length - 1, i + 1));
  };
  // The dialog itself holds focus when the gallery opens, so the arrow keys are read from the
  // window rather than from a control inside it. The key towards the previous picture's button
  // goes back, which is the right arrow when the page is laid out right to left.
  useEffect(() => {
    const back = direction === "rtl" ? "ArrowRight" : "ArrowLeft";
    const forward = direction === "rtl" ? "ArrowLeft" : "ArrowRight";
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === back) {
        event.preventDefault();
        setIndex((i) => Math.max(0, i - 1));
      } else if (event.key === forward) {
        event.preventDefault();
        setIndex((i) => Math.min(pictures.length - 1, i + 1));
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [pictures.length, direction]);

  const body = useRef<HTMLDivElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const [slide, setSlide] = useState<Slide>({ kind: "still" });
  const stageWidth = () => stage.current?.offsetWidth ?? window.innerWidth;
  /** Where the pictures are shown for a finger `travel` from the current one's place. */
  const shownTravel = (travel: number) => {
    const pastAnEnd = (travel > 0 && index === 0) || (travel < 0 && index === pictures.length - 1);
    return pastAnEnd ? rubberBand(travel, stageWidth()) : travel;
  };
  // Where motion is reduced the finger still moves the pictures, and lifting it puts them in
  // their places at once rather than sliding them there.
  const atOnce = motion.off || motion.reduced;
  const settleFrom = (shown: number, toward: -1 | 0 | 1) => {
    if (toward !== 0) {
      setIndex(index + toward);
    }
    if (atOnce || shown === 0) {
      setSlide({ kind: "still" });
    } else if (toward === 0) {
      setSlide({ kind: "easing" });
    } else {
      // The new current picture is drawn where the finger left it, and eases into place.
      setSlide({ kind: "placed", travel: shown + toward * stageWidth() });
    }
  };
  useSwipe(
    body,
    {
      onStart: () => pictures.length > 1 && slide.kind !== "placed",
      onMove: (travel) => {
        setSlide({ kind: "dragging", travel: shownTravel(travel) });
      },
      onEnd: (travel, velocity) => {
        settleFrom(
          shownTravel(travel),
          swipeOutcome(travel, velocity, stageWidth(), {
            hasPrevious: index > 0,
            hasNext: index < pictures.length - 1,
          }),
        );
      },
      onCancel: () => {
        settleFrom(slide.kind === "dragging" ? slide.travel : 0, 0);
      },
    },
    touchScreen && pictures.length > 1,
  );
  // Drawn first where the finger left them, so that they slide into place from there.
  useEffect(() => {
    if (slide.kind !== "placed") {
      return;
    }
    void stage.current?.getBoundingClientRect();
    const frame = requestAnimationFrame(() => {
      setSlide({ kind: "easing" });
    });
    return () => {
      cancelAnimationFrame(frame);
    };
  }, [slide.kind]);
  const onSlid = (event: TransitionEvent<HTMLDivElement>) => {
    if (event.propertyName === "transform") {
      setSlide((s) => (s.kind === "easing" ? { kind: "still" } : s));
    }
  };

  // The gallery fills the page, so the overlay's own click-outside never fires; a click that
  // lands on nothing in particular closes it instead. The press must start here too: the
  // control that opened the gallery fires its press on pointer-up, and the browser's click
  // event then lands on whatever the gallery has just rendered under the pointer.
  const pressedBackground = useRef(false);
  const isBackground = (target: EventTarget) =>
    target instanceof Element && target.closest("button, a, img") === null;
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    pressedBackground.current = isBackground(event.target);
  };
  const onBackgroundClick = (event: MouseEvent<HTMLDivElement>) => {
    if (pressedBackground.current && isBackground(event.target)) {
      close();
    }
    pressedBackground.current = false;
  };
  if (current === undefined) {
    return null;
  }
  const currentCovered = isCovered(current);
  // The current picture, and on a touch screen its neighbours beside it, ready to be swiped
  // in; the travel and the neighbours' sides are along the inline axis, so each is flipped
  // where the page is laid out right to left.
  const flip = direction === "rtl" ? -1 : 1;
  const travel = slide.kind === "dragging" || slide.kind === "placed" ? slide.travel : 0;
  const shown = touchScreen ? [index - 1, index, index + 1] : [index];
  return (
    <div
      ref={body}
      role="group"
      aria-label={m.gallery.label}
      onPointerDown={onPointerDown}
      onClick={onBackgroundClick}
      className="flex h-full w-full flex-col gap-3 p-4 text-white"
    >
      <div className="flex items-center justify-between">
        <span className="text-sm tabular-nums">
          {format(m.gallery.counter, {
            index: String(index + 1),
            count: String(pictures.length),
          })}
        </span>
        <span className="flex gap-2">
          <Link
            href={current.src}
            target="_blank"
            rel="noreferrer noopener"
            aria-label={m.gallery.open}
            className={navButtonClass}
          >
            <ArrowSquareOutIcon size={20} aria-hidden="true" />
          </Link>
          <Button onPress={close} aria-label={m.gallery.close} className={navButtonClass}>
            <XIcon size={20} aria-hidden="true" />
          </Button>
        </span>
      </div>
      <div className="flex min-h-0 flex-1 items-center gap-3">
        {!touchOnly && (
          <Button
            onPress={previous}
            isDisabled={index === 0}
            aria-label={m.gallery.previous}
            className={navButtonClass}
          >
            <CaretLeftIcon size={24} aria-hidden="true" className="rtl:-scale-x-100" />
          </Button>
        )}
        {/* Stretched so its height is definite and each picture's `max-h-full` has something to
            resolve against; centred only, the stage would take the picture's own height and a
            tall photo would spill over the controls above it. */}
        <div
          ref={stage}
          onTransitionEnd={onSlid}
          className="relative min-h-0 min-w-0 flex-1 self-stretch overflow-hidden"
        >
          {shown.map((i) => {
            const picture = pictures[i];
            if (picture === undefined) {
              return null;
            }
            const isCurrent = i === index;
            const covered = isCovered(picture);
            const image = (
              <img
                src={picture.src}
                alt={
                  isCurrent && !covered
                    ? pictureAlt(picture, format(m.imageAlt, { name: picture.name }))
                    : ""
                }
                referrerPolicy="no-referrer"
                className={
                  "max-h-full max-w-full object-contain" + (covered && !isCurrent ? " blur-md" : "")
                }
              />
            );
            return (
              <div
                key={picture.src + String(i)}
                aria-hidden={isCurrent ? undefined : true}
                className="absolute inset-0 flex items-center justify-center"
                style={{
                  transform: `translateX(calc(${String((i - index) * flip * 100)}% + ${String(travel * flip)}px))`,
                  transition:
                    slide.kind === "easing"
                      ? "transform var(--motion-slow) var(--motion-ease-out)"
                      : "none",
                }}
              >
                {isCurrent && covered ? (
                  <SpoilerCover
                    onReveal={() => {
                      if (picture.attachmentId !== undefined) {
                        onReveal(picture.attachmentId);
                      }
                    }}
                    className="flex max-h-full max-w-full"
                  >
                    {image}
                  </SpoilerCover>
                ) : (
                  image
                )}
              </div>
            );
          })}
        </div>
        {!touchOnly && (
          <Button
            onPress={next}
            isDisabled={index === pictures.length - 1}
            aria-label={m.gallery.next}
            className={navButtonClass}
          >
            <CaretRightIcon size={24} aria-hidden="true" className="rtl:-scale-x-100" />
          </Button>
        )}
      </div>
      {current.description != null && !currentCovered && (
        // Its uploader's description, for every reader; the picture's text alternative already
        // carries it to assistive technology, so it is not read twice.
        <p
          aria-hidden="true"
          className="mx-auto max-h-24 max-w-2xl overflow-y-auto text-center text-sm whitespace-pre-wrap"
        >
          {current.description}
        </p>
      )}
      <ul className="flex justify-center gap-2 overflow-x-auto py-1">
        {pictures.map((picture, i) => (
          <li key={picture.src + String(i)}>
            <Button
              onPress={() => {
                setIndex(i);
              }}
              aria-label={
                isCovered(picture)
                  ? m.gallery.spoiler
                  : pictureAlt(picture, format(m.imageAlt, { name: picture.name }))
              }
              aria-pressed={i === index}
              className={
                "block h-14 w-14 overflow-hidden rounded-md outline-none focus-visible:ring-2 focus-visible:ring-white/70 " +
                (i === index ? "ring-2 ring-white" : "opacity-60 hover:opacity-100")
              }
            >
              {/* A thumbnail: the inline copy does, where there is one, at a fraction of the
                  original's bytes. */}
              <img
                src={picture.preview?.src ?? picture.src}
                alt=""
                loading="lazy"
                referrerPolicy="no-referrer"
                className={"h-full w-full object-cover" + (isCovered(picture) ? " blur-md" : "")}
              />
            </Button>
          </li>
        ))}
      </ul>
    </div>
  );
}
