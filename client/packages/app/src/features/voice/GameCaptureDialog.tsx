import { CaretDownIcon } from "@phosphor-icons/react";
import { useEffect, useMemo, useState } from "react";
import {
  Button,
  Dialog,
  Label,
  ListBox,
  ListBoxItem,
  Modal,
  ModalOverlay,
  Popover,
  Select,
  SelectValue,
} from "react-aria-components";
import { useSync } from "@/api/hooks";
import { primaryButtonClass } from "@/features/auth/styles";
import {
  dialogClass,
  modalClass,
  optionClass,
  overlayClass,
  selectButtonClass,
} from "@/features/invites/dialog";
import {
  applicationAudioShare,
  captureChoice,
  gameCaptureShare,
  testPattern,
  type AudioKind,
  type AudioTarget,
  type CaptureCatalogue,
  type CaptureChoice,
  type GameCaptureBridge,
} from "@/features/voice/gameCapture";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

interface Option {
  readonly id: string;
  readonly label: string;
  readonly choice: CaptureChoice;
}

/** No application's sound chosen. */
const NO_AUDIO = "none";

/**
 * What the desktop shell can share as a game. Where libobs captures games (Windows and macOS)
 * the dialog lists the windows its capture source can be pointed at, with a checkbox to take
 * the window's own sound too; choosing one starts the capture. On Linux the picture comes from
 * the browser's screen share, whose picker is the system's, and the helper captures one
 * application's sound: the dialog lists the applications playing sound, and its button opens
 * the system picker for the picture. Choosing sound and picture separately is a limitation of
 * the Linux desktop portal, which only the app owning a window can raise and which reports
 * nothing about the application behind the window chosen. In development a test pattern is
 * offered too.
 */
export function GameCaptureDialog({
  bridge,
  onClose,
}: {
  bridge: GameCaptureBridge;
  onClose: () => void;
}) {
  const m = useMessages();
  const sync = useSync();
  const [catalogue, setCatalogue] = useState<CaptureCatalogue | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [starting, setStarting] = useState(false);
  const [withAudio, setWithAudio] = useState(true);
  const [audioChoice, setAudioChoice] = useState<string>(NO_AUDIO);

  useEffect(() => {
    let cancelled = false;
    bridge.kinds().then(
      (loaded) => {
        if (cancelled) {
          return;
        }
        setCatalogue(loaded);
        // The one application playing is the likely game; more than one needs a choice.
        if (loaded.applicationAudio?.targets?.length === 1) {
          setAudioChoice("0");
        }
      },
      (failure: unknown) => {
        if (!cancelled) {
          setCatalogue({ kinds: [], applicationAudio: null, testMedia: null });
          setError(failure instanceof Error ? failure.message : String(failure));
        }
      },
    );
    return () => {
      cancelled = true;
    };
  }, [bridge]);

  const options = useMemo(
    () => (catalogue === null ? null : optionsFor(catalogue, m)),
    [catalogue, m],
  );
  const applicationAudio = catalogue?.applicationAudio ?? null;
  const applications = applicationAudio?.targets ?? null;
  const windowAudio = catalogue?.kinds.some((kind) => kind.audio !== null) ?? false;
  const noAudio =
    catalogue !== null && catalogue.kinds.length > 0 && !windowAudio && applicationAudio === null;

  const run = async (share: () => Promise<void>) => {
    setStarting(true);
    setError(null);
    try {
      await share();
      onClose();
    } catch (failure) {
      // Dismissing the system picker is an ordinary outcome, not an error to show.
      if (!(failure instanceof DOMException && failure.name === "NotAllowedError")) {
        setError(failure instanceof Error ? failure.message : String(failure));
      }
      setStarting(false);
    }
  };

  const captureGame = (choice: CaptureChoice) =>
    run(() =>
      sync.voice.startExternalScreenShare(
        gameCaptureShare(bridge, choice, withAudio, () => {
          sync.voice.stopScreenShare();
        }),
      ),
    );

  const shareWithApplicationAudio = (kind: AudioKind, targets: readonly AudioTarget[]) => {
    const application = audioChoice === NO_AUDIO ? undefined : targets[Number(audioChoice)];
    // A game moves: its picture should keep its frame rate when bandwidth runs short.
    return run(() =>
      sync.voice.startScreenShare(
        application === undefined
          ? { contentHint: "motion" }
          : {
              contentHint: "motion",
              audio: applicationAudioShare(bridge, kind, application, () => {
                sync.voice.stopScreenShare();
              }),
            },
      ),
    );
  };

  return (
    <ModalOverlay
      isOpen
      onOpenChange={(open) => {
        if (!open) {
          onClose();
        }
      }}
      isDismissable
      className={overlayClass}
    >
      <Modal className={modalClass}>
        <Dialog className={dialogClass}>
          <DialogHeading>{m.voice.shareGameHeading}</DialogHeading>
          {catalogue === null && (
            <p className="text-sm text-ink-muted">{m.voice.shareGameLoading}</p>
          )}
          {applicationAudio !== null && applications !== null && (
            <>
              <p className="text-sm text-ink-muted">{m.voice.shareGameTwoSteps}</p>
              <AudioSelect
                applications={applications}
                value={audioChoice}
                onChange={setAudioChoice}
              />
              <Button
                isDisabled={starting}
                onPress={() => {
                  void shareWithApplicationAudio(applicationAudio, applications);
                }}
                className={primaryButtonClass}
              >
                {m.voice.shareGameChooseWindow}
              </Button>
            </>
          )}
          {options !== null && options.length > 0 && (
            <ListBox
              aria-label={m.voice.shareGameHeading}
              items={options}
              selectionMode="single"
              onAction={(key) => {
                const option = options.find((o) => o.id === key);
                if (option !== undefined && !starting) {
                  void captureGame(option.choice);
                }
              }}
              className="max-h-80 overflow-y-auto rounded-md border border-line"
            >
              {(option) => (
                <ListBoxItem
                  id={option.id}
                  textValue={option.label}
                  className="cursor-default px-3 py-2 text-sm outline-none focus:bg-surface-hover hover:bg-surface-hover"
                >
                  {option.label}
                </ListBoxItem>
              )}
            </ListBox>
          )}
          {options !== null && options.length === 0 && applicationAudio === null && (
            <p className="text-sm text-ink-muted">{m.voice.shareGameNone}</p>
          )}
          {windowAudio && (
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={withAudio}
                onChange={(event) => {
                  setWithAudio(event.target.checked);
                }}
                className="h-4 w-4 accent-accent"
              />
              {m.voice.shareGameAudio}
            </label>
          )}
          {noAudio && <p className="text-xs text-ink-muted">{m.voice.shareGameNoAudio}</p>}
          {error !== null && (
            <p role="alert" className="text-sm text-danger">
              {error}
            </p>
          )}
        </Dialog>
      </Modal>
    </ModalOverlay>
  );
}

/** The applications playing sound, with "No audio", for the sharer to pick which to send. */
function AudioSelect({
  applications,
  value,
  onChange,
}: {
  applications: readonly AudioTarget[];
  value: string;
  onChange: (value: string) => void;
}) {
  const m = useMessages();
  const items = [
    { id: NO_AUDIO, label: m.voice.shareGameAudioNone },
    ...applications.map((application, index) => ({
      id: String(index),
      label: applicationLabel(application, m),
    })),
  ];
  return (
    <Select
      value={value}
      onChange={(key) => {
        onChange(String(key));
      }}
      className="flex flex-col gap-1"
    >
      <Label className="text-sm font-medium">{m.voice.shareGameAudioLabel}</Label>
      <Button className={selectButtonClass}>
        <SelectValue className="truncate" />
        <CaretDownIcon size={14} aria-hidden="true" className="shrink-0 text-ink-faint" />
      </Button>
      <Popover className="min-w-(--trigger-width) rounded-md border border-line bg-surface-raised p-1 shadow-lg">
        <ListBox items={items}>
          {(item) => (
            <ListBoxItem id={item.id} textValue={item.label} className={optionClass}>
              {item.label}
            </ListBoxItem>
          )}
        </ListBox>
      </Popover>
      {applications.length === 0 && (
        <p className="text-xs text-ink-muted">{m.voice.shareGameAudioNobody}</p>
      )}
    </Select>
  );
}

function applicationLabel(application: AudioTarget, m: ReturnType<typeof useMessages>): string {
  const name = application.name.trim();
  if (name !== "") {
    return name;
  }
  if (application.pid !== null) {
    return format(m.voice.shareGameAudioProcess, { pid: String(application.pid) });
  }
  return m.voice.shareGameAudioUnknown;
}

function optionsFor(catalogue: CaptureCatalogue, m: ReturnType<typeof useMessages>): Option[] {
  const options: Option[] = [];
  for (const kind of catalogue.kinds) {
    for (const target of kind.targets) {
      options.push({
        id: `${kind.kind}:${target.value}`,
        label: target.name,
        choice: captureChoice(kind, target),
      });
    }
  }
  if (import.meta.env.DEV) {
    options.push({
      id: "test-pattern",
      label: m.voice.shareGameTestPattern,
      choice: testPattern(catalogue.testMedia),
    });
  }
  return options;
}
