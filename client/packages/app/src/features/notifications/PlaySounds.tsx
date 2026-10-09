import { useContext, useEffect } from "react";
import { useSources } from "@/api/everywhere";
import { HomeSyncContext } from "@/api/syncContext";
import { notificationOutputDevice, voiceOutputDevice } from "@/features/settings/audioDevices";
import { watchCallSounds } from "@/features/voice/callSounds";
import { watchConnectionSounds } from "./connectionSounds";
import { playSound } from "./sounds";

/**
 * Plays the app's sounds on every deployment the user is signed in to: the call's
 * (`watchCallSounds`), through the voice chat's speaker, and a deployment lost
 * (`watchConnectionSounds`), through the notification sound's.
 */
export function PlaySounds() {
  const home = useContext(HomeSyncContext);
  const sources = useSources();
  useEffect(() => {
    if (home === null) {
      return;
    }
    const stops = sources.flatMap(({ sync }) => [
      watchCallSounds(sync, (sound) => {
        void playSound(sound, voiceOutputDevice(home.preferences));
      }),
      watchConnectionSounds(sync, (sound) => {
        void playSound(sound, notificationOutputDevice(home.preferences));
      }),
    ]);
    return () => {
      for (const stop of stops) {
        stop();
      }
    };
  }, [home, sources]);
  return null;
}
