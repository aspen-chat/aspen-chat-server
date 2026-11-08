import {
  AUDIO_INPUT,
  AUDIO_OUTPUT,
  DEFAULT_DEVICE,
  INPUT_VOLUME,
  OUTPUT_VOLUME,
  SPEAKERPHONE,
  VIDEO_INPUT,
  canChooseOutput,
  resolveDevice,
} from "@aspen/protocol";
import { ArrowsClockwiseIcon, CaretUpIcon, SpeakerHighIcon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { Button, Dialog, DialogTrigger, Popover, ToggleButton } from "react-aria-components";
import { usePreference, useSync, useVoiceCall } from "@/api/hooks";
import { choosesSpeakerphone, usePhoneOutputs } from "@/api/audioRoute";
import { isMobilePlatform } from "@/features/layout/platform";
import { Tooltip } from "@/features/layout/Tooltip";
import {
  type CameraDevice,
  type CameraFacing,
  flippedCamera,
  flipsBetweenSides,
} from "@/features/settings/audioDevices";
import { AccessNotice, DeviceSelect } from "@/features/settings/DeviceSelect";
import { useAudioDevices } from "@/features/settings/useAudioDevices";
import { VolumeSlider } from "@/features/voice/VolumeSlider";
import { useMessages } from "@/i18n/context";

const rowClass =
  "flex items-center gap-2 rounded-md p-1 text-sm font-medium outline-none hover:bg-surface-hover " +
  "pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50";

/**
 * A call bar button with the small arrow at its end that opens `children` in a popover above
 * it. The arrow lies over the button's own padding, out of the layout, so the button keeps the
 * size and place it has without one.
 */
export function WithDeviceArrow({
  label,
  control,
  children,
}: {
  label: string;
  control: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="relative flex min-w-0 flex-1">
      {control}
      <DialogTrigger>
        <Tooltip text={label}>
          <Button
            aria-label={label}
            className="absolute inset-y-0 end-0 my-auto flex h-6 w-3.5 items-center justify-center rounded text-ink-faint outline-none hover:bg-surface-hover hover:text-ink pressed:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent/50"
          >
            <CaretUpIcon size={12} weight="bold" aria-hidden="true" />
          </Button>
        </Tooltip>
        <Popover
          placement="top"
          className="w-72 max-w-[calc(100vw-2rem)] rounded-lg border border-line bg-surface-raised p-3 shadow-lg"
        >
          <Dialog aria-label={label} className="flex flex-col gap-3 outline-none">
            {children}
          </Dialog>
        </Popover>
      </DialogTrigger>
    </div>
  );
}

/**
 * The microphone the call sends and how loud. On a phone or tablet with one microphone there is
 * nothing to choose, and only the volume shows.
 */
export function InputDevices() {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  // Only while the call holds the microphone is asking for it silent.
  const { devices, access, requestAccess } = useAudioDevices(
    call.status === "connected" && call.canSpeak,
  );
  const gain = usePreference(INPUT_VOLUME);
  return (
    <>
      <AccessNotice kind="microphone" access={access.microphone} onAllow={requestAccess} />
      {(!isMobilePlatform() || devices.inputs.length > 1) && (
        <DeviceSelect
          label={m.settings.microphone}
          definition={AUDIO_INPUT}
          devices={devices.inputs}
          defaults={[{ id: DEFAULT_DEVICE, label: m.settings.systemDefault }]}
        />
      )}
      <VolumeSlider
        label={m.voice.inputVolume}
        gain={gain}
        onChange={(value) => {
          void sync.preferences.set(INPUT_VOLUME, value);
        }}
      />
    </>
  );
}

/**
 * Where the call plays and how loud. A phone with only its receiver and loudspeaker toggles
 * between the two; elsewhere on a phone or tablet the speaker is chosen only where there are
 * several and the browser can choose, and a computer shows the choice as Settings does.
 */
export function OutputDevices() {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  const { devices } = useAudioDevices(call.status === "connected" && call.canSpeak);
  const phone = usePhoneOutputs();
  const gain = usePreference(OUTPUT_VOLUME);
  const mobile = isMobilePlatform();
  const choosable = canChooseOutput();
  return (
    <>
      {mobile && choosesSpeakerphone(phone) ? (
        <SpeakerphoneToggle />
      ) : (
        (!mobile || (choosable && devices.outputs.length > 1)) && (
          <DeviceSelect
            label={m.settings.speaker}
            definition={AUDIO_OUTPUT}
            devices={choosable ? devices.outputs : []}
            defaults={[{ id: DEFAULT_DEVICE, label: m.settings.systemDefault }]}
            disabled={!choosable}
            hint={choosable ? undefined : m.settings.outputUnsupported}
          />
        )
      )}
      <VolumeSlider
        label={m.voice.outputVolume}
        gain={gain}
        onChange={(value) => {
          void sync.preferences.set(OUTPUT_VOLUME, value);
        }}
      />
    </>
  );
}

/** Plays the call through the loudspeaker while pressed, through the receiver while not. */
function SpeakerphoneToggle() {
  const m = useMessages();
  const sync = useSync();
  const on = usePreference(SPEAKERPHONE);
  return (
    <ToggleButton
      isSelected={on}
      onChange={(selected) => {
        void sync.preferences.set(SPEAKERPHONE, selected);
      }}
      className={rowClass}
    >
      <span
        className={
          "flex size-8 shrink-0 items-center justify-center rounded-full " +
          (on ? "forced-fill bg-accent text-accent-contrast" : "text-ink-muted")
        }
      >
        <SpeakerHighIcon size={20} aria-hidden="true" />
      </span>
      {m.voice.speakerphone}
    </ToggleButton>
  );
}

/**
 * The camera the call sends. A phone or tablet whose cameras all face front or back flips
 * between the two sides; anything else chooses from the list.
 */
export function CameraDevices() {
  const m = useMessages();
  const { devices, access, requestAccess } = useAudioDevices(false);
  const flips = isMobilePlatform() && flipsBetweenSides(devices.cameras);
  return (
    <>
      <AccessNotice kind="camera" access={access.camera} onAllow={requestAccess} />
      {flips ? (
        <CameraFlip cameras={devices.cameras} />
      ) : (
        <DeviceSelect
          label={m.settings.camera}
          definition={VIDEO_INPUT}
          devices={devices.cameras}
          defaults={[{ id: DEFAULT_DEVICE, label: m.settings.systemDefault }]}
        />
      )}
    </>
  );
}

/**
 * Switches to the main camera of the other side, as the camera choice, so the next call starts
 * on it too. The side in use is the chosen camera's, or the open camera's while the system's
 * default is in use, or the front, which a phone's default is.
 */
function CameraFlip({ cameras }: { cameras: readonly CameraDevice[] }) {
  const m = useMessages();
  const sync = useSync();
  const call = useVoiceCall();
  const chosen = usePreference(VIDEO_INPUT);
  const chosenId =
    chosen === DEFAULT_DEVICE
      ? null
      : resolveDevice(
          chosen,
          cameras.map((camera) => ({ deviceId: camera.id, label: camera.label })),
        );
  const open = call.localCamera?.getSettings().facingMode;
  const facing: CameraFacing =
    cameras.find((camera) => camera.id === chosenId)?.facing ??
    (open === "environment" ? "back" : "front");
  const target = flippedCamera(cameras, facing);
  if (target === undefined) {
    return null;
  }
  return (
    <Button
      onPress={() => {
        void sync.preferences.set(VIDEO_INPUT, { id: target.id, label: target.label });
      }}
      className={rowClass}
    >
      <span className="flex size-8 shrink-0 items-center justify-center rounded-full text-ink-muted">
        <ArrowsClockwiseIcon size={20} aria-hidden="true" />
      </span>
      {target.facing === "back" ? m.voice.useBackCamera : m.voice.useFrontCamera}
    </Button>
  );
}
