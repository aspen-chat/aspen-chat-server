import { useEffect, useState } from "react";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { type AudioDevices, sortDevices } from "@/features/settings/audioDevices";

/**
 * The microphones, speakers, and cameras the browser can offer, kept current as devices come and go.
 * Browsers name devices only once the page holds a media permission, so the first read asks
 * for the microphone and lets it go again; a refusal leaves the lists unnamed but usable.
 */
export function useAudioDevices(): {
  devices: AudioDevices;
  permission: "unknown" | "granted" | "denied";
} {
  const m = useMessages();
  const [devices, setDevices] = useState<AudioDevices>({ inputs: [], outputs: [], cameras: [] });
  const [permission, setPermission] = useState<"unknown" | "granted" | "denied">("unknown");
  useEffect(() => {
    if (typeof navigator === "undefined" || !("mediaDevices" in navigator)) {
      return;
    }
    let cancelled = false;
    const unnamedInput = (index: number) =>
      format(m.settings.unnamedMicrophone, { index: String(index) });
    const unnamedOutput = (index: number) =>
      format(m.settings.unnamedSpeaker, { index: String(index) });
    const unnamedCamera = (index: number) =>
      format(m.settings.unnamedCamera, { index: String(index) });
    const refresh = async () => {
      const listed = await navigator.mediaDevices.enumerateDevices();
      if (!cancelled) {
        const sorted = sortDevices(listed, (index) => unnamedInput(index));
        setDevices({
          inputs: sorted.inputs,
          outputs: sortDevices(listed, unnamedOutput).outputs,
          cameras: sortDevices(listed, unnamedCamera).cameras,
        });
      }
    };
    const unlock = async () => {
      try {
        const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
        for (const track of stream.getTracks()) {
          track.stop();
        }
        if (!cancelled) {
          setPermission("granted");
        }
      } catch {
        if (!cancelled) {
          setPermission("denied");
        }
      }
      await refresh();
    };
    void unlock();
    const onChange = () => {
      void refresh();
    };
    navigator.mediaDevices.addEventListener("devicechange", onChange);
    return () => {
      cancelled = true;
      navigator.mediaDevices.removeEventListener("devicechange", onChange);
    };
  }, [m]);
  return { devices, permission };
}
