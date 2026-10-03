import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { type AudioDevices, sortDevices } from "@/features/settings/audioDevices";

/** A media permission that unlocks a list of devices: microphones and speakers, or cameras. */
export type DeviceKind = "microphone" | "camera";

/**
 * Whether a list of devices can be shown: `granted` once the browser names them, `locked` until
 * the user is asked, `denied` once the user or the system refused, and `absent` when the request
 * found no such device.
 */
export type DeviceAccess = "locked" | "granted" | "denied" | "absent";

const MEDIA_KIND: Record<DeviceKind, MediaDeviceKind> = {
  microphone: "audioinput",
  camera: "videoinput",
};

function mediaDevicesAvailable(): boolean {
  return typeof navigator !== "undefined" && "mediaDevices" in navigator;
}

/**
 * The microphones, speakers, and cameras the browser can offer, kept current as devices come and go.
 * Browsers name devices only once the page holds a media permission, and asking for one shows the
 * user a prompt, so the microphone is asked for on its own only during a call, when the call already
 * holds it and the request is answered silently. Otherwise the lists are read as they are, a list the
 * browser will not name is left empty, and `requestAccess` asks when the user chooses to.
 * Speakers are named under the microphone's permission, as browsers do.
 */
export function useAudioDevices(inCall: boolean): {
  devices: AudioDevices;
  access: Record<DeviceKind, DeviceAccess>;
  requestAccess: (kind: DeviceKind) => Promise<void>;
} {
  const m = useMessages();
  const [listed, setListed] = useState<readonly MediaDeviceInfo[]>([]);
  const [refused, setRefused] = useState<Partial<Record<DeviceKind, "denied" | "absent">>>({});
  const mounted = useRef(false);

  const refresh = useCallback(async () => {
    if (!mediaDevicesAvailable()) {
      return;
    }
    const devices = await navigator.mediaDevices.enumerateDevices();
    if (mounted.current) {
      setListed(devices);
    }
  }, []);

  const requestAccess = useCallback(
    async (kind: DeviceKind) => {
      let refusal: "denied" | "absent" | undefined;
      try {
        const stream = await navigator.mediaDevices.getUserMedia(
          kind === "microphone" ? { audio: true } : { video: true },
        );
        for (const track of stream.getTracks()) {
          track.stop();
        }
      } catch (error) {
        refusal =
          error instanceof DOMException && error.name === "NotFoundError" ? "absent" : "denied";
      }
      if (mounted.current) {
        setRefused((current) => ({ ...current, [kind]: refusal }));
      }
      await refresh();
    },
    [refresh],
  );

  useEffect(() => {
    if (!mediaDevicesAvailable()) {
      return;
    }
    mounted.current = true;
    void (inCall ? requestAccess("microphone") : refresh());
    const onChange = () => {
      void refresh();
    };
    navigator.mediaDevices.addEventListener("devicechange", onChange);
    return () => {
      mounted.current = false;
      navigator.mediaDevices.removeEventListener("devicechange", onChange);
    };
  }, [inCall, refresh, requestAccess]);

  return useMemo(() => {
    const accessTo = (kind: DeviceKind): DeviceAccess => {
      const ofKind = listed.filter((d) => d.kind === MEDIA_KIND[kind]);
      if (ofKind.some((d) => d.label !== "")) {
        return "granted";
      }
      // A device plugged in since the request found none is worth asking for again.
      const refusal = refused[kind];
      return refusal === "absent" && ofKind.length > 0 ? "locked" : (refusal ?? "locked");
    };
    const access = { microphone: accessTo("microphone"), camera: accessTo("camera") };
    const named = (message: string) => (index: number) => format(message, { index: String(index) });
    const sorted = sortDevices(listed, named(m.settings.unnamedMicrophone));
    return {
      devices: {
        inputs: access.microphone === "granted" ? sorted.inputs : [],
        outputs:
          access.microphone === "granted"
            ? sortDevices(listed, named(m.settings.unnamedSpeaker)).outputs
            : [],
        cameras:
          access.camera === "granted"
            ? sortDevices(listed, named(m.settings.unnamedCamera)).cameras
            : [],
      },
      access,
      requestAccess,
    };
  }, [listed, refused, m, requestAccess]);
}
