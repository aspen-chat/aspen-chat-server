import { ArrowSquareOutIcon, CaretLeftIcon, CaretRightIcon, XIcon } from "@phosphor-icons/react";
import { useEffect, useRef, useState, type MouseEvent, type PointerEvent } from "react";
import { Button, Dialog, Link, Modal, ModalOverlay, useLocale } from "react-aria-components";
import { pictureAlt, type Picture } from "@/features/messages/images";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

const navButtonClass =
  "rounded-full bg-black/50 p-2 text-white outline-none hover:bg-black/70 pressed:bg-black/80 " +
  "disabled:opacity-30 focus-visible:ring-2 focus-visible:ring-white/70";

/**
 * Every picture of a message, one at a time, over a darkened page, with its uploader's
 * description beneath it when it has one: arrows and the arrow keys move between them, a strip of thumbnails jumps to one, and Escape, the close control, or a
 * click on the empty space around the picture leaves. Opens on the picture at `initial`.
 */
export function ImageGallery({
  pictures,
  initial,
  isOpen,
  onClose,
}: {
  pictures: readonly Picture[];
  initial: number;
  isOpen: boolean;
  onClose: () => void;
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
      className="fixed inset-0 z-20 flex items-center justify-center bg-black/80 motion-backdrop"
    >
      <Modal className="h-full w-full outline-none motion-dialog">
        <Dialog aria-label={m.gallery.label} className="flex h-full w-full flex-col outline-none">
          {({ close }) => <GalleryBody pictures={pictures} initial={initial} close={close} />}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function GalleryBody({
  pictures,
  initial,
  close,
}: {
  pictures: readonly Picture[];
  initial: number;
  close: () => void;
}) {
  const m = useMessages();
  const { direction } = useLocale();
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
  return (
    <div
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
        <Button
          onPress={previous}
          isDisabled={index === 0}
          aria-label={m.gallery.previous}
          className={navButtonClass}
        >
          <CaretLeftIcon size={24} aria-hidden="true" className="rtl:-scale-x-100" />
        </Button>
        {/* Stretched so its height is definite and the picture's `max-h-full` has something to
            resolve against; centred only, the wrapper would take the picture's own height and a
            tall photo would spill over the controls above it. */}
        <div className="flex min-h-0 min-w-0 flex-1 items-center justify-center self-stretch overflow-hidden">
          <img
            key={current.src}
            src={current.src}
            alt={pictureAlt(current, format(m.imageAlt, { name: current.name }))}
            referrerPolicy="no-referrer"
            className="max-h-full max-w-full object-contain"
          />
        </div>
        <Button
          onPress={next}
          isDisabled={index === pictures.length - 1}
          aria-label={m.gallery.next}
          className={navButtonClass}
        >
          <CaretRightIcon size={24} aria-hidden="true" className="rtl:-scale-x-100" />
        </Button>
      </div>
      {current.description != null && (
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
              aria-label={pictureAlt(picture, format(m.imageAlt, { name: picture.name }))}
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
                className="h-full w-full object-cover"
              />
            </Button>
          </li>
        ))}
      </ul>
    </div>
  );
}
