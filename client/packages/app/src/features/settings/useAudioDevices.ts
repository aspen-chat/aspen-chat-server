import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AspenCall, nativeCalls, type PhoneCamera } from "@/api/nativeCall";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import {
  type AudioDevices,
  type CameraDevice,
  sortDevices,
} from "@/features/settings/audioDevices";

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
 * The page's standing permission for `kind`, kept current, or `undefined` where the browser
 * cannot say. Querying it never shows a prompt, unlike `getUserMedia`. The desktop app grants
 * the permission without asking, so a list it does not name (no such device is plugged in) is
 * still not worth a button that asks.
 */
function usePermissionState(kind: DeviceKind): PermissionState | undefined {
  const [state, setState] = useState<PermissionState>();
  useEffect(() => {
    if (typeof navigator === "undefined" || !("permissions" in navigator)) {
      return;
    }
    let status: PermissionStatus | undefined;
    let current = true;
    const onChange = () => {
      if (status !== undefined) {
        setState(status.state);
      }
    };
    navigator.permissions.query({ name: kind }).then(
      (answer) => {
        if (!current) {
          return;
        }
        status = answer;
        setState(answer.state);
        answer.addEventListener("change", onChange);
      },
      // A browser that does not know the permission's name refuses the query, and is left to
      // say through the device list alone.
      () => undefined,
    );
    return () => {
      current = false;
      status?.removeEventListener("change", onChange);
    };
  }, [kind]);
  return state;
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
  const phoneCameras = usePhoneCameras();
  const [refused, setRefused] = useState<Partial<Record<DeviceKind, "denied" | "absent">>>({});
  const mounted = useRef(false);
  const microphonePermission = usePermissionState("microphone");
  const cameraPermission = usePermissionState("camera");

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
      const standing = kind === "microphone" ? microphonePermission : cameraPermission;
      if (standing === "granted") {
        return ofKind.length > 0 ? "granted" : "absent";
      }
      if (standing === "denied") {
        return "denied";
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
          phoneCameras ??
          (access.camera === "granted"
            ? sortDevices(listed, named(m.settings.unnamedCamera)).cameras
            : []),
      },
      access:
        phoneCameras === null
          ? access
          : { ...access, camera: phoneCameras.length > 0 ? "granted" : "absent" },
      requestAccess,
    };
  }, [listed, refused, microphonePermission, cameraPermission, m, requestAccess, phoneCameras]);
}

/**
 * The phone's cameras where the native call engine carries the call (`nativeCalls`), as the
 * system lists them through the plugin, each with which way it faces as the system says and
 * named by that where the system gives no name; `null` everywhere else, where the browser
 * lists them. The system lists them without asking the user, who is asked as the camera
 * turns on, so they count as granted.
 */
function usePhoneCameras(): readonly CameraDevice[] | null {
  const m = useMessages();
  const [cameras, setCameras] = useState<readonly PhoneCamera[] | null>(null);
  const native = nativeCalls();
  useEffect(() => {
    if (!native) {
      return;
    }
    let current = true;
    AspenCall.cameras().then(
      ({ cameras: listed }) => {
        if (current) {
          setCameras(listed);
        }
      },
      (error: unknown) => {
        console.error(error);
      },
    );
    return () => {
      current = false;
    };
  }, [native]);
  return useMemo(() => {
    if (!native) {
      return null;
    }
    return (cameras ?? []).map((camera, index) => ({
      id: camera.id,
      label:
        camera.label ??
        (camera.facing === "front"
          ? m.settings.frontCamera
          : camera.facing === "back"
            ? m.settings.backCamera
            : format(m.settings.unnamedCamera, { index: String(index + 1) })),
      facing: camera.facing,
    }));
  }, [native, cameras, m]);
}
