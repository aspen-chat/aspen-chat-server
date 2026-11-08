import { SPEAKERPHONE } from "@aspen/protocol";
import { Capacitor, registerPlugin, type PluginListenerHandle } from "@capacitor/core";
import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { useCallSource } from "./calls";

/**
 * What a phone can play a call through: its receiver (the earpiece held to the ear, which an
 * iPad and most tablets lack), its loudspeaker, and anything else connected (headphones, a
 * headset, a car), which the system routes a call to on its own.
 */
export interface PhoneOutputs {
  readonly receiver: boolean;
  readonly speaker: boolean;
  readonly other: boolean;
}

/**
 * The mobile apps' native side of a call's output (`AspenAudioRoute`, in each): web code can
 * neither list a phone's receiver and loudspeaker nor choose between them. `setSpeakerphone`
 * holds the choice until `release`, which gives the route back to the system; `outputsChange`
 * tells when something is connected or taken away.
 */
interface AspenAudioRoutePlugin {
  outputs(): Promise<PhoneOutputs>;
  setSpeakerphone(options: { on: boolean }): Promise<void>;
  release(): Promise<void>;
  addListener(
    event: "outputsChange",
    listener: (outputs: PhoneOutputs) => void,
  ): Promise<PluginListenerHandle>;
}

const AspenAudioRoute = registerPlugin<AspenAudioRoutePlugin>("AspenAudioRoute");

function canRoute(): boolean {
  return Capacitor.isNativePlatform() && Capacitor.isPluginAvailable("AspenAudioRoute");
}

/** Whether the choice is the receiver or the loudspeaker alone: nothing else is connected. */
export function choosesSpeakerphone(outputs: PhoneOutputs | null): boolean {
  return outputs !== null && outputs.receiver && outputs.speaker && !outputs.other;
}

/** The phone's outputs, kept current; `null` outside the mobile apps, which cannot tell. */
export function usePhoneOutputs(): PhoneOutputs | null {
  const [outputs, setOutputs] = useState<PhoneOutputs | null>(null);
  useEffect(() => {
    if (!canRoute()) {
      return;
    }
    let live = true;
    let handle: PluginListenerHandle | null = null;
    void AspenAudioRoute.addListener("outputsChange", (changed) => {
      if (live) {
        setOutputs(changed);
      }
    }).then((added) => {
      if (live) {
        handle = added;
      } else {
        void added.remove();
      }
    });
    void AspenAudioRoute.outputs().then(
      (read) => {
        if (live) {
          setOutputs(read);
        }
      },
      () => undefined,
    );
    return () => {
      live = false;
      void handle?.remove();
    };
  }, []);
  return outputs;
}

/**
 * In the mobile apps, plays a call through the loudspeaker or the receiver as `SPEAKERPHONE`
 * says, from when the call starts until it ends, and again whenever the choice or what is
 * connected changes. While anything else is connected the system's own route is left alone.
 */
export function useFollowSpeakerphone(): void {
  const source = useCallSource();
  const outputs = usePhoneOutputs();
  const preferences = source?.sync.preferences ?? null;
  const inCall = source !== null && source.sync.voice.state.status !== "idle";
  const subscribe = useCallback(
    (listener: () => void) => preferences?.subscribe(listener) ?? (() => undefined),
    [preferences],
  );
  const speakerphone = useSyncExternalStore(
    subscribe,
    () => preferences?.get(SPEAKERPHONE) ?? SPEAKERPHONE.fallback,
  );
  const holds = inCall && choosesSpeakerphone(outputs);
  useEffect(() => {
    if (!canRoute()) {
      return;
    }
    const routed = holds
      ? AspenAudioRoute.setSpeakerphone({ on: speakerphone })
      : AspenAudioRoute.release();
    void routed.catch(() => undefined);
  }, [holds, speakerphone, outputs]);
}
