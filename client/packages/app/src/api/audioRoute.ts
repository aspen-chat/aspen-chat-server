import { SPEAKERPHONE } from "@aspen/protocol";
import type { PluginListenerHandle } from "@capacitor/core";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { useCallSource } from "./calls";
import { AspenCall, nativeCalls, type PhoneRoutes } from "./nativeCall";

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
 * The outputs as the call's typed routes (`AspenCall.routes`) say: the receiver where the
 * device has one, the loudspeaker always, and anything of another kind among the routes. Web
 * code can neither list a phone's receiver and loudspeaker nor choose between them, so both
 * come from the native side.
 */
export function outputsOf(routes: PhoneRoutes): PhoneOutputs {
  return {
    receiver: routes.hasReceiver,
    speaker: true,
    other: routes.routes.some((route) => route.kind !== "receiver" && route.kind !== "loudspeaker"),
  };
}

/** Whether the choice is the receiver or the loudspeaker alone: nothing else is connected. */
export function choosesSpeakerphone(outputs: PhoneOutputs | null): boolean {
  return outputs !== null && outputs.receiver && outputs.speaker && !outputs.other;
}

/** The phone's outputs, kept current; `null` outside the mobile apps, which cannot tell. */
export function usePhoneOutputs(): PhoneOutputs | null {
  const [outputs, setOutputs] = useState<PhoneOutputs | null>(null);
  useEffect(() => {
    if (!nativeCalls()) {
      return;
    }
    let live = true;
    let handle: PluginListenerHandle | null = null;
    void AspenCall.addListener("routes", (changed) => {
      if (live) {
        setOutputs(outputsOf(changed));
      }
    }).then((added) => {
      if (live) {
        handle = added;
      } else {
        void added.remove();
      }
    });
    void AspenCall.routes().then(
      (read) => {
        if (live) {
          setOutputs(outputsOf(read));
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
 * connected changes: the choice goes to the call engine's routes (`AspenCall.setRoute`), which
 * move the call's own audio. The choice is withdrawn (`clearRoute`) when the call it was made
 * for ends, and while anything else is connected, so the system's own route applies; a choice
 * the page did not make (one the native side holds, or a test's) is never withdrawn, so outside
 * a call, where the page applies none, it clears none either, whatever the routes say.
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
  /** Whether the page's choice stands on the native side, so there is one to withdraw. */
  const applied = useRef(false);
  useEffect(() => {
    if (!nativeCalls()) {
      return;
    }
    let routed: Promise<void>;
    if (holds) {
      applied.current = true;
      routed = AspenCall.setRoute({ kind: speakerphone ? "loudspeaker" : "receiver" });
    } else if (applied.current) {
      applied.current = false;
      routed = AspenCall.clearRoute();
    } else {
      return;
    }
    routed.catch((error: unknown) => {
      console.error(error);
    });
  }, [holds, speakerphone, outputs]);
}
