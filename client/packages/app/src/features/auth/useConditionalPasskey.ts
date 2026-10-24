import {
  ApiProblemError,
  conditionalMediationAvailable,
  type PasskeyTransport,
} from "@aspen/protocol";
import { useCallback, useEffect, useRef } from "react";
import { useAspenClient } from "@/api/context";

/** What the sign-in form does with a conditional passkey sign-in. */
export interface ConditionalPasskey {
  /**
   * Ends the one waiting, before the form signs in another way: a browser runs one passkey
   * request at a time, and one left waiting would answer a pick after the person moved on.
   */
  stop: () => void;
  /**
   * Starts another where none is waiting, as the person returns to the username field: after
   * one expired, failed, or was stopped. Each start is a ceremony the server counts toward the
   * address's and device's sign-in limits, so starts follow what the person does, never a
   * timer: a page left open starts one and then none until someone uses it.
   */
  rearm: () => void;
}

/**
 * Passkey autofill on the sign-in form: a `signIn` ceremony with conditional mediation, which
 * offers the browser's passkeys for this deployment among the username field's suggestions
 * (`autocomplete="username webauthn"`) and signs in with the one picked. Only a web client
 * under the relying party's domain runs it; the apps hand ceremonies elsewhere and have no
 * such field. `paused` holds off `rearm` while the form signs in another way. A refusal from
 * the server is reported to `onFailure`; an ending the person chose, or expiry, is not.
 */
export function useConditionalPasskey(
  transport: PasskeyTransport | null,
  paused: boolean,
  onFailure: (message: string) => void,
): ConditionalPasskey {
  const client = useAspenClient();
  const rpId = transport?.kind === "inPage" ? transport.rpId : null;
  const waiting = useRef<AbortController | null>(null);
  const available = useRef(false);
  const pausedNow = useRef(paused);
  const report = useRef(onFailure);
  useEffect(() => {
    pausedNow.current = paused;
    report.current = onFailure;
  });

  const start = useCallback(() => {
    if (rpId === null || !available.current || waiting.current !== null || pausedNow.current) {
      return;
    }
    const controller = new AbortController();
    waiting.current = controller;
    client
      .runConditionalPasskeySignIn(rpId, controller.signal)
      .catch((e: unknown) => {
        if (e instanceof ApiProblemError && !controller.signal.aborted) {
          report.current(e.message);
        }
      })
      .finally(() => {
        if (waiting.current === controller) {
          waiting.current = null;
        }
      });
  }, [client, rpId]);

  const stop = useCallback(() => {
    waiting.current?.abort();
    waiting.current = null;
  }, []);

  useEffect(() => {
    if (rpId === null) {
      return;
    }
    let live = true;
    void conditionalMediationAvailable().then((yes) => {
      available.current = yes;
      if (live) {
        start();
      }
    });
    return () => {
      live = false;
      stop();
    };
  }, [rpId, start, stop]);

  return { stop, rearm: start };
}
