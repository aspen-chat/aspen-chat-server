import { DEFAULT_DEVICE, VIDEO_INPUT, resolveDevice } from "@aspen/protocol";
import { usePreference, useVoiceCall } from "@/api/hooks";
import { choosesSpeakerphone, usePhoneOutputs } from "@/api/audioRoute";
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
