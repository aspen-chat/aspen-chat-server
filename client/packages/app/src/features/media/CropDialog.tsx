import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from "react";
import {
  Button,
  Dialog,
  Label,
  Modal,
  ModalOverlay,
  Slider,
  SliderThumb,
  SliderTrack,
} from "react-aria-components";
import { primaryButtonClass } from "@/features/auth/styles";
import { dialogClass, modalClass, overlayClass } from "@/features/invites/dialog";
import {
  boundingSquare,
  clampCircle,
  fitScale,
  iconSize,
  initialCircle,
  maxRadius,
  MIN_RADIUS,
  type Circle,
} from "@/features/media/crop";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";

/** The largest the picture is shown at inside the dialog. */
const PREVIEW_MAX = 360;

/**
 * Lets the reader choose the part of a picture to use as an icon: a circle they can drag
 * anywhere inside the picture and resize with a slider, never crossing an edge. Confirming
 * crops the picture to the square around the circle and hands back a PNG of it.
 */
export function CropDialog({
  file,
  onCrop,
  onCancel,
}: {
  file: File;
  onCrop: (icon: Blob) => void;
  onCancel: () => void;
}) {
  const m = useMessages();
  const [image, setImage] = useState<HTMLImageElement | null>(null);
  const [failed, setFailed] = useState(false);
  const [circle, setCircle] = useState<Circle | null>(null);

  useEffect(() => {
    const url = URL.createObjectURL(file);
    const picture = new Image();
    // Revoking the URL below aborts a load still under way, which reports as an error; a load
    // this effect has already given up on must not mark the file unreadable.
    let cancelled = false;
    picture.onload = () => {
      if (!cancelled) {
        setImage(picture);
        setCircle(initialCircle(picture.naturalWidth, picture.naturalHeight));
      }
    };
    picture.onerror = () => {
      if (!cancelled) {
        setFailed(true);
      }
    };
    picture.src = url;
    return () => {
      cancelled = true;
      URL.revokeObjectURL(url);
    };
  }, [file]);

  function confirm() {
    if (image === null || circle === null) {
      return;
    }
    const square = boundingSquare(circle);
    const size = iconSize(circle);
    const canvas = document.createElement("canvas");
    canvas.width = size;
    canvas.height = size;
    const context = canvas.getContext("2d");
    if (context === null) {
      setFailed(true);
      return;
    }
    context.drawImage(image, square.x, square.y, square.size, square.size, 0, 0, size, size);
    canvas.toBlob((blob) => {
      if (blob === null) {
        setFailed(true);
      } else {
        onCrop(blob);
      }
    }, "image/png");
  }

  return (
    <ModalOverlay
      isOpen
      onOpenChange={(open) => {
        if (!open) {
          onCancel();
        }
      }}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>{m.crop.heading}</DialogHeading>
          <p className="text-sm text-ink-muted">{m.crop.hint}</p>
          {failed ? (
            <p role="alert" className="text-sm text-danger">
              {m.crop.unreadable}
            </p>
          ) : image === null || circle === null ? (
            <div className="flex h-48 items-center justify-center text-sm text-ink-muted">
              {m.loading}
            </div>
          ) : (
            <CropSurface
              image={image}
              circle={circle}
              onChange={(next) => {
                setCircle(clampCircle(next, image.naturalWidth, image.naturalHeight));
              }}
            />
          )}
          <div className="flex justify-end gap-2">
            <Button
              onPress={confirm}
              isDisabled={image === null || failed}
              className={primaryButtonClass}
            >
              {m.crop.use}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/** The picture with the selection over it; dragging moves the circle, the slider sizes it. */
function CropSurface({
  image,
  circle,
  onChange,
}: {
  image: HTMLImageElement;
  circle: Circle;
  onChange: (circle: Circle) => void;
}) {
  const m = useMessages();
  const width = image.naturalWidth;
  const height = image.naturalHeight;
  const scale = fitScale(width, height, PREVIEW_MAX, PREVIEW_MAX);
  const shownWidth = width * scale;
  const shownHeight = height * scale;
  /** Where the pointer took hold of the circle, relative to its centre, in image pixels. */
  const grip = useRef<{ dx: number; dy: number } | null>(null);
  const surface = useRef<HTMLDivElement>(null);

  function imagePoint(event: ReactPointerEvent): { x: number; y: number } {
    const rect = surface.current?.getBoundingClientRect();
    if (rect === undefined) {
      return { x: circle.x, y: circle.y };
    }
    return { x: (event.clientX - rect.left) / scale, y: (event.clientY - rect.top) / scale };
  }

  function onPointerDown(event: ReactPointerEvent<HTMLDivElement>) {
    const point = imagePoint(event);
    grip.current = { dx: point.x - circle.x, dy: point.y - circle.y };
    event.currentTarget.setPointerCapture(event.pointerId);
    // A press outside the circle moves it there at once; a press inside keeps its grip.
    if (Math.hypot(point.x - circle.x, point.y - circle.y) > circle.radius) {
      grip.current = { dx: 0, dy: 0 };
      onChange({ ...circle, x: point.x, y: point.y });
    }
  }

  function onPointerMove(event: ReactPointerEvent<HTMLDivElement>) {
    if (grip.current === null) {
      return;
    }
    const point = imagePoint(event);
    onChange({ ...circle, x: point.x - grip.current.dx, y: point.y - grip.current.dy });
  }

  function onPointerUp() {
    grip.current = null;
  }

  const maskId = "crop-mask";
  return (
    <div className="flex flex-col items-center gap-3">
      <div
        ref={surface}
        role="img"
        aria-label={m.crop.previewLabel}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={onPointerUp}
        className="relative touch-none cursor-move overflow-hidden rounded-md bg-surface-sunken select-none"
        style={{ width: shownWidth, height: shownHeight }}
      >
        <img
          src={image.src}
          alt=""
          draggable={false}
          className="block"
          style={{ width: shownWidth, height: shownHeight }}
        />
        <svg
          className="absolute inset-0"
          width={shownWidth}
          height={shownHeight}
          viewBox={`0 0 ${String(shownWidth)} ${String(shownHeight)}`}
          aria-hidden="true"
        >
          <defs>
            <mask id={maskId}>
              <rect width="100%" height="100%" fill="white" />
              <circle
                cx={circle.x * scale}
                cy={circle.y * scale}
                r={circle.radius * scale}
                fill="black"
              />
            </mask>
          </defs>
          <rect width="100%" height="100%" fill="rgba(0, 0, 0, 0.55)" mask={`url(#${maskId})`} />
          <circle
            cx={circle.x * scale}
            cy={circle.y * scale}
            r={circle.radius * scale}
            fill="none"
            stroke="white"
            strokeWidth={2}
          />
        </svg>
      </div>
      <Slider
        value={circle.radius}
        minValue={MIN_RADIUS}
        maxValue={maxRadius(width, height)}
        step={1}
        onChange={(value) => {
          if (typeof value === "number") {
            onChange({ ...circle, radius: value });
          }
        }}
        className="flex w-full flex-col gap-1"
      >
        <Label className="text-sm font-medium text-ink-muted">{m.crop.size}</Label>
        <SliderTrack className="relative h-6 w-full">
          <div className="absolute top-1/2 h-1 w-full -translate-y-1/2 rounded-full bg-line" />
          <SliderThumb className="top-1/2 h-4 w-4 rounded-full border border-line bg-accent outline-none dragging:bg-accent-strong focus-visible:ring-2 focus-visible:ring-accent/50" />
        </SliderTrack>
      </Slider>
    </div>
  );
}
