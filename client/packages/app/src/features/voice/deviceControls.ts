import { type Call, DEFAULT_DEVICE, VIDEO_INPUT, resolveDevice } from "@aspen/protocol";
import { usePreference, useSync, useVoiceCall } from "@/api/hooks";
import { type PhoneOutputs, choosesSpeakerphone, usePhoneOutputs } from "@/api/audioRoute";
import { isMobilePlatform } from "@/features/layout/platform";
import {
  type CameraDevice,
  type CameraFacing,
  flippedCamera,
  flipsBetweenSides,
  trackFacing,
} from "@/features/settings/audioDevices";
import { useAudioDevices } from "@/features/settings/useAudioDevices";

/**
 * Whether the bar's Speaker button shows: in the mobile apps, on a phone with only its receiver
 * and loudspeaker to play a call through.
 */
export function useSpeakerphoneButton(): boolean {
  return choosesSpeakerphone(usePhoneOutputs()) && isMobilePlatform();
}

/** What the device controls ask of the call: whether it takes a microphone or speaker by id. */
export type DeviceChoosing = Pick<Call, "hasAudioDeviceChoice">;

/**
 * Whether the microphone is chosen from a list (`InputDevices`, Settings' Audio and video):
 * only where the call takes one by id (`Call.hasAudioDeviceChoice`; the mobile apps' native
 * calls send the phone's own and take none), and then on a computer always, on a phone or
 * tablet only when it has several, since with one there is nothing to choose.
 */
export function listsMicrophones(call: DeviceChoosing, mobile: boolean, inputs: number): boolean {
  return call.hasAudioDeviceChoice && (!mobile || inputs > 1);
}

/**
 * Whether the speaker is chosen from a list (`OutputDevices`): only where the call takes one
 * by id (`Call.hasAudioDeviceChoice`; the mobile apps' native calls play where the phone's
 * routes say), and then on a computer always (the select disabled where the browser cannot
 * choose), on a phone or tablet only where the browser can choose among several and the bar's
 * Speaker button does not choose between the receiver and loudspeaker instead.
 */
export function listsSpeakers(
  call: DeviceChoosing,
  mobile: boolean,
  phone: PhoneOutputs | null,
  choosable: boolean,
  outputs: number,
): boolean {
  return (
    call.hasAudioDeviceChoice &&
    (!mobile || (!choosesSpeakerphone(phone) && choosable && outputs > 1))
  );
}

/**
 * Whether the mute button has options to open (`InputDevices`): the microphone's volume where
 * the call applies one (`Call.hasInputVolume`), and on a computer the choice of microphone
 * where the call takes one (`Call.hasAudioDeviceChoice`). A phone whose call the phone's own
 * engine carries has neither: its own microphone is sent as captured, and where the call plays
 * is the routes', so the arrow is left off.
 */
export function useMicrophoneOptions(): boolean {
  const sync = useSync();
  return sync.voice.hasInputVolume || (sync.voice.hasAudioDeviceChoice && !isMobilePlatform());
}

/**
 * The camera a flip switches to, on a phone or tablet whose cameras all face front or back:
 * the main camera of the other side (`flippedCamera`). The side in use is the chosen camera's,
 * or the open camera's while the system's default is in use, or the front, which a phone's
 * default is. `undefined` where the cameras do not flip, or are not named yet.
 */
export function useCameraFlip(): CameraDevice | undefined {
  const call = useVoiceCall();
  const { devices } = useAudioDevices(false);
  const chosen = usePreference(VIDEO_INPUT);
  const cameras = devices.cameras;
  if (!isMobilePlatform() || !flipsBetweenSides(cameras)) {
    return undefined;
  }
  const chosenId =
    chosen === DEFAULT_DEVICE
      ? null
      : resolveDevice(
          chosen,
          cameras.map((camera) => ({ deviceId: camera.id, label: camera.label })),
        );
  const facing: CameraFacing =
    cameras.find((camera) => camera.id === chosenId)?.facing ??
    (call.localCamera === null ? null : trackFacing(call.localCamera)) ??
    "front";
  return flippedCamera(cameras, facing);
}
