import { useEffect, useRef, useState } from "react";
import { Dialog, Modal, ModalOverlay } from "react-aria-components";
import { modalClass, dialogClass, overlayClass } from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";

/** How often a frame is read while nothing is found. */
const READ_EVERY_MS = 200;
/** Frames are scaled down to this width before reading: a code fills enough of it to read. */
const READ_WIDTH = 640;

/**
 * The camera, reading QR codes until `onScan` accepts one. `onScan` returns `null` to accept the
 * code (and the dialog closes), or what to tell the reader about a code that is not the kind
 * wanted, and scanning goes on. The reader (`zxing.ts`, WebAssembly served with the app) loads
 * only when the dialog first opens.
 */
export function QrScannerDialog({
  isOpen,
  onOpenChange,
  title,
  hint,
  onScan,
}: {
  isOpen: boolean;
  onOpenChange: (open: boolean) => void;
  title: string;
  /** What to point the camera at, and any warning about it. */
  hint: string;
  onScan: (text: string) => string | null;
}) {
  return (
    <ModalOverlay
      isOpen={isOpen}
      onOpenChange={onOpenChange}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>{title}</DialogHeading>
          <p className="text-sm text-ink-muted">{hint}</p>
          <Camera
            onScan={(text) => {
              const refusal = onScan(text);
              if (refusal === null) {
                onOpenChange(false);
              }
              return refusal;
            }}
          />
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function Camera({ onScan }: { onScan: (text: string) => string | null }) {
  const m = useMessages();
  const video = useRef<HTMLVideoElement>(null);
  const [failure, setFailure] = useState<string | null>(null);
  const [refusal, setRefusal] = useState<string | null>(null);
  const [starting, setStarting] = useState(true);
  const accept = useRef(onScan);
  useEffect(() => {
    accept.current = onScan;
  });

  useEffect(() => {
    // Read through a function, since what it guards comes after awaits, during which the
    // dialog may close.
    const live = { stopped: false };
    const stopped = () => live.stopped;
    let stream: MediaStream | null = null;
    let timer: number | undefined;
    let lastRefused: string | null = null;
    const canvas = document.createElement("canvas");
    const context = canvas.getContext("2d", { willReadFrequently: true });

    async function run() {
      try {
        stream = await navigator.mediaDevices.getUserMedia({
          video: { facingMode: { ideal: "environment" } },
          audio: false,
        });
      } catch (e) {
        if (!live.stopped) {
          setStarting(false);
          setFailure(
            e instanceof DOMException && e.name === "NotAllowedError"
              ? m.qr.cameraDenied
              : m.qr.cameraUnavailable,
          );
        }
        return;
      }
      if (live.stopped) {
        stream.getTracks().forEach((track) => {
          track.stop();
        });
        return;
      }
      const element = video.current;
      if (element === null || context === null) {
        return;
      }
      element.srcObject = stream;
      await element.play().catch(() => undefined);
      const { readQrCode } = await import("./zxing");
      setStarting(false);
      const read = async () => {
        if (live.stopped) {
          return;
        }
        if (element.videoWidth > 0) {
          const scale = Math.min(1, READ_WIDTH / element.videoWidth);
          canvas.width = Math.round(element.videoWidth * scale);
          canvas.height = Math.round(element.videoHeight * scale);
          context.drawImage(element, 0, 0, canvas.width, canvas.height);
          const text = await readQrCode(context.getImageData(0, 0, canvas.width, canvas.height));
          // The same refused code stays in view for many frames; it is answered once.
          if (!stopped() && text !== null && text !== lastRefused) {
            const answer = accept.current(text);
            if (answer === null) {
              live.stopped = true;
              return;
            }
            lastRefused = text;
            setRefusal(answer);
          }
        }
        timer = window.setTimeout(() => void read(), READ_EVERY_MS);
      };
      void read();
    }

    void run();
    return () => {
      live.stopped = true;
      window.clearTimeout(timer);
      stream?.getTracks().forEach((track) => {
        track.stop();
      });
    };
  }, [m]);

  if (failure !== null) {
    return (
      <p role="alert" className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">
        {failure}
      </p>
    );
  }
  return (
    <div className="flex flex-col gap-2">
      <div
        className="relative aspect-square w-full overflow-hidden rounded-lg bg-black"
        aria-busy={starting}
        aria-label={starting ? m.qr.cameraStarting : m.qr.cameraLabel}
        role="img"
      >
        <video ref={video} muted playsInline className="size-full object-cover" />
        {/* The square to aim at, a guide only: the whole frame is read. */}
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-[15%] rounded-lg border-2 border-white/80"
        />
      </div>
      {refusal !== null && (
        <p role="alert" className="text-sm text-danger">
          {refusal}
        </p>
      )}
    </div>
  );
}
