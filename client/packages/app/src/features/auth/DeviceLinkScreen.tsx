import { ApiProblemError, type AspenClient, type DeviceLinkScan } from "@aspen/protocol";
import { Link, useLocation } from "@tanstack/react-router";
import { type ReactNode, useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { primaryButtonClass } from "@/features/auth/styles";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { Caution } from "./DeviceLinkCode";
import { thisDeviceName } from "./deviceName";
import { useServerChoice } from "./serverChoice";
import { deviceLinkOf } from "@/features/qr/aspenLinks";
import { usePasskeyTransport } from "@/features/auth/passkeyTransport";
import { ReauthProvider } from "@/features/security/reauth";
import { useOptionalReauth } from "@/features/security/reauthContext";
import { useSecuritySettings } from "@/features/security/useSecuritySettings";

/** How often a phone waiting to be signed in asks, within the server's limit. */
const POLL_MS = 2_000;

/**
 * Each code scanned by this page, by id: scanning uses a code up, so a screen that mounts twice
 * (React's development double render, or the route rendering again once signed in) reuses the
 * one scan rather than meeting `deviceLinkUsed` from its own first.
 */
const scans = new Map<string, Promise<{ scan: DeviceLinkScan; verifier: string | null }>>();
/** Codes this page was signed in with, so the signed-in route does not take them for new ones. */
const claimed = new Set<string>();

function scanOnce(client: AspenClient, id: string, deviceName: string) {
  let scan = scans.get(id);
  if (scan === undefined) {
    scan = client.scanDeviceLink(id, deviceName);
    scans.set(id, scan);
    // A scan refused before it used the code (a sign-in to verify again first) may be tried
    // again, so it is not kept.
    scan.catch((e: unknown) => {
      if (e instanceof ApiProblemError && e.code === "reauthenticationRequired") {
        scans.delete(id);
      }
    });
  }
  return scan;
}

type Phase =
  | { kind: "scanning" }
  /** Signed in: a computer asks to be signed in as this account. */
  | { kind: "confirm"; deviceName: string }
  | { kind: "approved"; deviceName: string }
  /** Signed out: waiting for the computer to confirm this phone. */
  | { kind: "waiting"; account: string; verifier: string }
  | { kind: "failed"; message: string };

function messageOf(e: unknown): string {
  return e instanceof ApiProblemError ? e.message : String(e);
}

/**
 * What a scanned sign-in code (`/device-link`) does on the phone. Signed in, it is a computer
 * asking for this account: the phone names it, warns that a stranger's code is a trap, and signs
 * it in only when confirmed. Signed out, it is a computer offering its account: the phone waits
 * for the computer to confirm it, then is signed in. A code for another server is refused when
 * signed in; signed out in the apps, the app offers to move to that server, naming its host, and
 * moves only when the person agrees. Giving a sign-in takes a recent verification, which it asks
 * for first when the server wants one.
 */
export function DeviceLinkScreen({ server, id }: { server: string; id: string }) {
  const m = useMessages();
  const client = useAspenClient();
  const { switchServer } = useServerChoice();
  const signedIn = client.session !== null;
  const elsewhere = client.baseUrl !== server;
  const [phase, setPhase] = useState<Phase>({ kind: "scanning" });
  const withReauth = useOptionalReauth();

  useEffect(() => {
    if (elsewhere || claimed.has(id)) {
      return;
    }
    let stopped = false;
    const name = thisDeviceName((app, system) =>
      system === null ? app : format(m.deviceLink.deviceName, { app, system }),
    );
    withReauth(() => scanOnce(client, id, name)).then(
      (scanned) => {
        if (stopped) {
          return;
        }
        if (scanned === undefined) {
          setPhase({ kind: "failed", message: m.deviceLink.notVerified });
          return;
        }
        const { scan, verifier } = scanned;
        if (scan.kind === "request" && signedIn) {
          setPhase({ kind: "confirm", deviceName: scan.deviceName ?? "" });
        } else if (scan.kind === "offer" && verifier !== null) {
          setPhase({
            kind: "waiting",
            account: scan.displayName ?? scan.userName ?? "",
            verifier,
          });
        }
      },
      (e: unknown) => {
        if (stopped) {
          return;
        }
        // A computer asking for a sign-in needs a phone that has one to give.
        setPhase({
          kind: "failed",
          message:
            e instanceof ApiProblemError && e.code === "unauthorized"
              ? m.deviceLink.needsSignedInPhone
              : messageOf(e),
        });
      },
    );
    return () => {
      stopped = true;
    };
  }, [client, id, elsewhere, signedIn, m, withReauth]);

  const verifier = phase.kind === "waiting" ? phase.verifier : null;
  useEffect(() => {
    if (verifier === null) {
      return;
    }
    let stopped = false;
    const timer = window.setInterval(() => {
      client.claimDeviceLink(id, verifier).then(
        (result) => {
          if (result.status === "signedIn") {
            // The session is stored; the signed-in app takes over from here.
            claimed.add(id);
            window.clearInterval(timer);
          }
        },
        (e: unknown) => {
          window.clearInterval(timer);
          if (!stopped) {
            setPhase({ kind: "failed", message: messageOf(e) });
          }
        },
      );
    }, POLL_MS);
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, [client, id, verifier]);

  if (claimed.has(id) && signedIn) {
    return <Finished text={m.deviceLink.signedInHere} />;
  }
  if (elsewhere && (signedIn || switchServer === null)) {
    return (
      <Failed
        text={format(m.deviceLink.wrongServer, {
          server: hostOf(server),
          here: hostOf(client.baseUrl),
        })}
      />
    );
  }
  if (elsewhere && switchServer !== null) {
    const move = switchServer;
    // Signed out in an app, the code names the server to sign in to, but anyone can make a link
    // naming any server, so the app moves there only when the person agrees, shown its host.
    const host = hostOf(server);
    return (
      <Frame>
        <p className="text-base font-medium">
          {format(m.deviceLink.switchServerPrompt, { server: host, here: hostOf(client.baseUrl) })}
        </p>
        <Caution text={m.deviceLink.switchServerWarning} />
        <div className="flex flex-wrap justify-end gap-2">
          <Link to="/" className={secondaryButtonClass}>
            {m.deviceLink.back}
          </Link>
          <Button
            onPress={() => {
              move(server);
            }}
            className={primaryButtonClass}
          >
            {format(m.deviceLink.switchServer, { server: host })}
          </Button>
        </div>
      </Frame>
    );
  }
  switch (phase.kind) {
    case "scanning":
      return (
        <Frame>
          <div aria-busy="true" className="flex flex-col gap-2">
            <LoadingLabel text={m.deviceLink.reading} />
            <Skeleton className="h-5 w-3/4" />
            <Skeleton className="h-16 w-full rounded-md" />
          </div>
        </Frame>
      );
    case "confirm":
      return (
        <Frame>
          <p className="text-base font-medium">
            {format(m.deviceLink.confirmRequest, { device: phase.deviceName })}
          </p>
          <Caution text={m.deviceLink.confirmRequestWarning} />
          <div className="flex flex-wrap justify-end gap-2">
            <Link
              to="/"
              onClick={() => {
                void client.cancelDeviceLink(id).catch(() => undefined);
              }}
              className={secondaryButtonClass}
            >
              {m.deviceLink.decline}
            </Link>
            <Button
              onPress={() => {
                client.approveDeviceLink(id).then(
                  () => {
                    setPhase({ kind: "approved", deviceName: phase.deviceName });
                  },
                  (e: unknown) => {
                    setPhase({ kind: "failed", message: messageOf(e) });
                  },
                );
              }}
              className={primaryButtonClass}
            >
              {m.deviceLink.approve}
            </Button>
          </div>
        </Frame>
      );
    case "approved":
      return <Finished text={format(m.deviceLink.approvedRequest, { device: phase.deviceName })} />;
    case "waiting":
      return (
        <Frame>
          <p role="status" className="text-base">
            {format(m.deviceLink.waitingForComputer, { account: phase.account })}
          </p>
          <Button
            onPress={() => {
              void client.cancelDeviceLink(id).catch(() => undefined);
              setPhase({ kind: "failed", message: m.deviceLink.cancelled });
            }}
            className={secondaryButtonClass + " self-end"}
          >
            {m.deviceLink.cancel}
          </Button>
        </Frame>
      );
    case "failed":
      return <Failed text={phase.message} />;
  }
}

/**
 * The `/device-link` route, signed in or out: the code its query and fragment name (the app's
 * own scanner passes the id as `link`, since a route in a fragment cannot carry a fragment of its
 * own).
 */
export function DeviceLinkRoute() {
  const m = useMessages();
  const signedIn = useAspenClient().session !== null;
  const { searchStr, hash } = useLocation();
  const link = deviceLinkOf(searchStr.replace(/^\?/, ""), hash);
  if (link === null) {
    return <Failed text={m.deviceLink.incomplete} />;
  }
  const screen = <DeviceLinkScreen key={link.id} server={link.server} id={link.id} />;
  return signedIn ? <SignedInReauth>{screen}</SignedInReauth> : screen;
}

/** Asks a signed-in phone to confirm it's them when giving its sign-in needs it. */
function SignedInReauth({ children }: { children: ReactNode }) {
  const security = useSecuritySettings();
  const transport = usePasskeyTransport();
  return (
    <ReauthProvider settings={security.settings} transport={transport}>
      {children}
    </ReauthProvider>
  );
}

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

function Frame({ children }: { children: React.ReactNode }) {
  const m = useMessages();
  return (
    <section
      aria-labelledby="device-link-heading"
      className="mx-auto flex w-full max-w-sm flex-col gap-4 p-6"
    >
      <h1 id="device-link-heading" className="text-xl font-semibold">
        {m.deviceLink.screenHeading}
      </h1>
      {children}
    </section>
  );
}

function Finished({ text }: { text: string }) {
  const m = useMessages();
  return (
    <Frame>
      <p role="status" className="text-base">
        {text}
      </p>
      <Link to="/" className={primaryButtonClass + " self-end"}>
        {m.deviceLink.done}
      </Link>
    </Frame>
  );
}

function Failed({ text }: { text: string }) {
  const m = useMessages();
  return (
    <Frame>
      <p role="alert" className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">
        {text}
      </p>
      <Link to="/" className={secondaryButtonClass + " self-end"}>
        {m.deviceLink.back}
      </Link>
    </Frame>
  );
}
