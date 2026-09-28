import { useEffect, useState } from "react";
import { Button, Dialog, Heading, Modal, ModalOverlay } from "react-aria-components";
import {
  dialogClass,
  headingClass,
  overlayClass,
  secondaryButtonClass,
  wideModalClass,
} from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";

/** A screen or window the desktop shell can share, as it lists them. */
export interface ShareableSource {
  readonly id: string;
  readonly name: string;
  readonly kind: "screen" | "window";
  /** A data URL of a small picture of it. */
  readonly thumbnail: string;
  readonly icon: string | null;
}

export interface PickRequest {
  readonly sources: readonly ShareableSource[];
  /** Whether the platform can bring the system's audio along with a screen. */
  readonly systemAudio: boolean;
}

/**
 * The desktop shell's own picker for screen sharing, on platforms without a system one. The
 * main process sends the sources when the page asks to share a screen; picking one, or
 * dismissing, answers it, and the share then starts with that source.
 */
export function SourcePickerDialog() {
  const m = useMessages();
  const [request, setRequest] = useState<PickRequest | null>(null);
  const [systemAudio, setSystemAudio] = useState(false);
  useEffect(() => {
    const picker = window.aspenDesktop?.displayPicker;
    if (picker === undefined) {
      return;
    }
    return picker.onPick((incoming) => {
      setRequest(incoming as PickRequest);
      setSystemAudio(false);
    });
  }, []);
  if (request === null) {
    return null;
  }
  const answer = (id: string | null) => {
    window.aspenDesktop?.displayPicker.choose({ id, systemAudio });
    setRequest(null);
  };
  const screens = request.sources.filter((source) => source.kind === "screen");
  const windows = request.sources.filter((source) => source.kind === "window");
  return (
    <ModalOverlay
      isOpen
      onOpenChange={(open) => {
        if (!open) {
          answer(null);
        }
      }}
      isDismissable
      className={overlayClass}
    >
      <Modal className={wideModalClass}>
        <Dialog className={dialogClass}>
          <Heading slot="title" className={headingClass}>
            {m.voice.pickSourceHeading}
          </Heading>
          <div className="flex max-h-[60vh] flex-col gap-4 overflow-y-auto">
            <SourceGroup heading={m.voice.pickScreens} sources={screens} onPick={answer} />
            <SourceGroup heading={m.voice.pickWindows} sources={windows} onPick={answer} />
          </div>
          <div className="flex items-center justify-between gap-2">
            {request.systemAudio ? (
              <label className="flex items-center gap-2 text-sm">
                <input
                  type="checkbox"
                  checked={systemAudio}
                  onChange={(event) => {
                    setSystemAudio(event.target.checked);
                  }}
                  className="h-4 w-4 accent-accent"
                />
                {m.voice.pickSystemAudio}
              </label>
            ) : (
              <span />
            )}
            <Button
              onPress={() => {
                answer(null);
              }}
              className={secondaryButtonClass}
            >
              {m.cancel}
            </Button>
          </div>
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

function SourceGroup({
  heading,
  sources,
  onPick,
}: {
  heading: string;
  sources: readonly ShareableSource[];
  onPick: (id: string) => void;
}) {
  if (sources.length === 0) {
    return null;
  }
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-sm font-semibold text-ink-muted">{heading}</h3>
      <ul className="grid grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-3">
        {sources.map((source) => (
          <li key={source.id}>
            <Button
              onPress={() => {
                onPick(source.id);
              }}
              className="flex w-full flex-col gap-1.5 rounded-md border border-line p-2 text-left outline-none hover:border-accent hover:bg-surface-hover pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
            >
              <img
                src={source.thumbnail}
                alt=""
                className="aspect-video w-full rounded bg-black object-contain"
              />
              <span className="flex min-w-0 items-center gap-1.5 text-sm">
                {source.icon !== null && <img src={source.icon} alt="" className="h-4 w-4" />}
                <span className="truncate">{source.name}</span>
              </span>
            </Button>
          </li>
        ))}
      </ul>
    </section>
  );
}
