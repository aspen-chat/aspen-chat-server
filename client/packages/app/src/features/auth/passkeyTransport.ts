import {
  canRunInPage,
  parseHandoffReturn,
  type AuthMethods,
  type HandoffReturn,
  type PasskeyHandoff,
  type PasskeyTransport,
} from "@aspen/protocol";
import { useEffect, useState } from "react";
import { useAspenClient } from "@/api/context";
import { detectShell, type Shell } from "@/config";
import { authMethods } from "@/features/auth/authMethods";

/**
 * The web client runs passkey ceremonies in its own page when it is served under the relying
 * party's domain; served anywhere else it cannot, and offers no passkeys. The desktop and mobile
 * shells never can, since their pages are local, so they hand ceremonies to the server's page
 * in the system browser, where the user's passkey managers and phone are.
 */
export function passkeyTransport(shell: Shell, methods: AuthMethods): PasskeyTransport | null {
  const rpId = methods.passkeys?.rpId;
  if (rpId === undefined) {
    return null;
  }
  switch (shell) {
    case "desktop": {
      const bridge = window.aspenDesktop?.passkeyHandoff;
      return bridge === undefined ? null : { kind: "handoff", handoff: desktopHandoff(bridge) };
    }
    case "mobile":
      return { kind: "handoff", handoff: mobileHandoff };
    case "web":
      return canRunInPage(rpId, window.location.hostname, "PublicKeyCredential" in window)
        ? { kind: "inPage" }
        : null;
  }
}

/** The main process listens on a loopback port for the browser's return. */
function desktopHandoff(bridge: AspenDesktopBridge["passkeyHandoff"]): PasskeyHandoff {
  return {
    async prepare() {
      const { id, returnTo } = await bridge.prepare();
      return {
        returnTo,
        open: (url) => bridge.open(id, url),
        dispose: () => {
          bridge.dispose(id);
        },
      };
    },
  };
}

/** The browser returns through the app's `aspen:` URL scheme, which the native projects claim. */
const mobileHandoff: PasskeyHandoff = {
  async prepare() {
    const [{ App }, { Browser }] = await Promise.all([
      import("@capacitor/app"),
      import("@capacitor/browser"),
    ]);
    let settle: (value: HandoffReturn) => void = () => undefined;
    const returned = new Promise<HandoffReturn>((resolve) => {
      settle = resolve;
    });
    const opened = await App.addListener("appUrlOpen", ({ url }) => {
      const result = parseHandoffReturn(url);
      if (result !== null) {
        settle(result);
        void Browser.close();
      }
    });
    // The user closed the in-app browser without finishing.
    const closed = await Browser.addListener("browserFinished", () => {
      settle({ ceremony: "", outcome: "cancelled", code: null });
    });
    return {
      returnTo: "aspen://auth/passkey",
      open: async (url) => {
        await Browser.open({ url });
        return returned;
      },
      dispose: () => {
        void opened.remove();
        void closed.remove();
      },
    };
  },
};

/**
 * How this shell reaches passkeys on the current server; `null` while unknown or when passkeys
 * are unavailable, in which case nothing passkey-related is offered.
 */
export function usePasskeyTransport(): PasskeyTransport | null {
  const client = useAspenClient();
  const [transport, setTransport] = useState<{
    server: string;
    value: PasskeyTransport | null;
  } | null>(null);
  useEffect(() => {
    let live = true;
    authMethods(client)
      .then((methods) => {
        if (live) {
          setTransport({ server: client.baseUrl, value: passkeyTransport(detectShell(), methods) });
        }
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, [client]);
  return transport?.server === client.baseUrl ? transport.value : null;
}
