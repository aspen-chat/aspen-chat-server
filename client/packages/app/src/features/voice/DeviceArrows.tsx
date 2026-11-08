import {
  AUDIO_INPUT,
  AUDIO_OUTPUT,
  DEFAULT_DEVICE,
  INPUT_VOLUME,
  OUTPUT_VOLUME,
  SPEAKERPHONE,
  VIDEO_INPUT,
  canChooseOutput,
} from "@aspen/protocol";
import { ArrowsClockwiseIcon, CaretUpIcon, SpeakerHighIcon } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { Button, Dialog, DialogTrigger, Popover, ToggleButton } from "react-aria-components";
import { usePreference, useSync, useVoiceCall } from "@/api/hooks";
import { choosesSpeakerphone, usePhoneOutputs } from "@/api/audioRoute";
import { isMobilePlatform } from "@/features/layout/platform";
import { Tooltip } from "@/features/layout/Tooltip";
import { type CameraDevice } from "@/features/settings/audioDevices";
import { AccessNotice, DeviceSelect } from "@/features/settings/DeviceSelect";
import { useAudioDevices } from "@/features/settings/useAudioDevices";
import { callBarButtonClass } from "@/features/voice/callBarButton";
import { VolumeSlider } from "@/features/voice/VolumeSlider";
import { useMessages } from "@/i18n/context";

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
 * Where the call plays and how loud. On a phone or tablet the speaker is chosen only where there
 * are several and the browser can choose, and not at all where the bar's Speaker button chooses
 * between a phone's receiver and loudspeaker; a computer shows the choice as Settings does.
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
  const lists = !mobile || (!choosesSpeakerphone(phone) && choosable && devices.outputs.length > 1);
  return (
    <>
      {lists && (
        <DeviceSelect
          label={m.settings.speaker}
          definition={AUDIO_OUTPUT}
          devices={choosable ? devices.outputs : []}
          defaults={[{ id: DEFAULT_DEVICE, label: m.settings.systemDefault }]}
          disabled={!choosable}
          hint={choosable ? undefined : m.settings.outputUnsupported}
        />
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

/**
 * Plays the call through the loudspeaker while pressed, through the receiver while not; its icon
 * sits in a filled accent circle while on.
 */
export function SpeakerphoneButton() {
  const m = useMessages();
  const sync = useSync();
  const on = usePreference(SPEAKERPHONE);
  return (
    <Tooltip text={m.voice.speakerphone}>
      <ToggleButton
        aria-label={m.voice.speakerphone}
        isSelected={on}
        onChange={(selected) => {
          void sync.preferences.set(SPEAKERPHONE, selected);
        }}
        className={callBarButtonClass}
      >
        {/* The circle is drawn past the icon's box, so the button is no taller than the rest. */}
        <span
          className={
            "-m-1 flex rounded-full p-1 " + (on ? "forced-fill bg-accent text-accent-contrast" : "")
          }
        >
          <SpeakerHighIcon size={24} aria-hidden="true" />
        </span>
      </ToggleButton>
    </Tooltip>
  );
}

/** The camera the call sends, from the list. */
export function CameraDevices() {
  const m = useMessages();
  const { devices, access, requestAccess } = useAudioDevices(false);
  return (
    <>
      <AccessNotice kind="camera" access={access.camera} onAllow={requestAccess} />
      <DeviceSelect
        label={m.settings.camera}
        definition={VIDEO_INPUT}
        devices={devices.cameras}
        defaults={[{ id: DEFAULT_DEVICE, label: m.settings.systemDefault }]}
      />
    </>
  );
}

/**
 * Switches to `target`, as the camera choice: a camera that is on moves to it and stays on, and
 * one that is off starts on it next.
 */
export function CameraFlipButton({ target }: { target: CameraDevice }) {
  const m = useMessages();
  const sync = useSync();
  const label = target.facing === "back" ? m.voice.useBackCamera : m.voice.useFrontCamera;
  return (
    <Tooltip text={label}>
      <Button
        aria-label={label}
        onPress={() => {
          void sync.preferences.set(VIDEO_INPUT, { id: target.id, label: target.label });
        }}
        className={callBarButtonClass}
      >
        <ArrowsClockwiseIcon size={24} aria-hidden="true" />
      </Button>
    </Tooltip>
  );
}
