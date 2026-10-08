import { ApiProblemError, type DeviceLink } from "@aspen/protocol";
import { ArrowClockwiseIcon, WarningIcon } from "@phosphor-icons/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { primaryButtonClass } from "@/features/auth/styles";
import { secondaryButtonClass } from "@/features/invites/dialog";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useNow } from "@/features/layout/useNow";
import { deviceLinkPath } from "@/features/qr/aspenLinks";
import { QrCode } from "@/features/qr/QrCode";
import { useShareUrl } from "@/features/qr/shareLinks";
import { useOptionalReauth } from "@/features/security/reauthContext";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { thisDeviceName } from "./deviceName";

/** How often the code's progress is asked while it shows, within the server's limit. */
const POLL_MS = 2_000;
/** How large the code is drawn: easy for a phone to read from arm's length. */
const CODE_PX = 224;

type Phase =
  | { kind: "starting" }
  | { kind: "showing"; link: DeviceLink; verifier: string | null }
  | { kind: "scanned"; link: DeviceLink; verifier: string | null; deviceName: string }
  | { kind: "approved"; deviceName: string }
  | { kind: "expired" }
  | { kind: "failed"; message: string };

/**
 * A sign-in code shown on a computer for a phone to scan (`app::device_link` on the server). Signed
 * out, it asks for a sign-in, and the computer is signed in once the phone confirms. Signed in, it
 * offers this account to a phone that is not, and this computer confirms the phone by its name.
 * The code lasts a minute; then it blurs and offers a new one. Whatever it ends with, a code left
 * behind is cancelled. Offering takes a recent verification, which it asks for where a
 * `ReauthProvider` holds it (the security settings).
 */
export function DeviceLinkCode() {
  const m = useMessages();
  const client = useAspenClient();
  const share = useShareUrl();
  const offering = client.session !== null;
  const withReauth = useOptionalReauth();
  const [phase, setPhase] = useState<Phase>({ kind: "starting" });
  const now = useNow(1_000, phase.kind === "showing");
  const live = useRef<string | null>(null);

  /** Makes a code, and what to show for it. */
  const makeCode = useCallback(async (): Promise<Phase> => {
    try {
      const name = thisDeviceName((app, system) =>
        system === null ? app : format(m.deviceLink.deviceName, { app, system }),
      );
      const made = await withReauth(() => client.startDeviceLink(name));
      if (made === undefined) {
        return { kind: "failed", message: m.deviceLink.notVerified };
      }
      return { kind: "showing", link: made.link, verifier: made.verifier };
    } catch (e) {
      return { kind: "failed", message: e instanceof ApiProblemError ? e.message : String(e) };
    }
  }, [client, m, withReauth]);

  /** Shows a code just made, which is the one to cancel if the screen goes. */
  const show = useCallback((made: Phase) => {
    live.current = made.kind === "showing" ? made.link.id : null;
    setPhase(made);
  }, []);

  useEffect(() => {
    let mounted = true;
    void makeCode().then((made) => {
      if (mounted) {
        show(made);
      } else if (made.kind === "showing") {
        // Made for a screen already gone (React's development double mount among them).
        void client.cancelDeviceLink(made.link.id).catch(() => undefined);
      }
    });
    return () => {
      mounted = false;
    };
  }, [client, makeCode, show]);

  // A code still waiting when the screen goes is ended, so it cannot be scanned afterwards.
  useEffect(
    () => () => {
      if (live.current !== null) {
        void client.cancelDeviceLink(live.current).catch(() => undefined);
      }
    },
    [client],
  );

  const waiting = phase.kind === "showing" || phase.kind === "scanned" ? phase : null;
  useEffect(() => {
    if (waiting === null) {
      return;
    }
    const { link, verifier } = waiting;
    let stopped = false;
    const ask = async () => {
      try {
        if (verifier !== null) {
          const claimed = await client.claimDeviceLink(link.id, verifier);
          // Signed in: the session is stored and the signed-in app replaces this screen.
          if (claimed.status === "signedIn") {
            live.current = null;
          } else if (claimed.status === "scanned" && !stopped) {
            setPhase({ kind: "scanned", link, verifier, deviceName: claimed.deviceName });
          }
        } else {
          const progress = await client.deviceLinkProgress(link.id);
          if (stopped) {
            return;
          }
          if (progress.status === "scanned") {
            setPhase((current) =>
              current.kind === "showing"
                ? { kind: "scanned", link, verifier, deviceName: progress.deviceName }
                : current,
            );
          }
        }
      } catch (e) {
        if (stopped) {
          return;
        }
        live.current = null;
        if (e instanceof ApiProblemError && e.code === "deviceLinkExpired") {
          setPhase({ kind: "expired" });
        } else {
          setPhase({
            kind: "failed",
            message: e instanceof ApiProblemError ? e.message : String(e),
          });
        }
      }
    };
    const timer = window.setInterval(() => void ask(), POLL_MS);
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, [client, waiting]);

  // Unscanned at its end, the code is gone on the server too.
  const expiresAt = phase.kind === "showing" ? Date.parse(phase.link.expiresAt) : null;
  const secondsLeft = expiresAt === null ? null : Math.max(0, Math.ceil((expiresAt - now) / 1000));
  useEffect(() => {
    if (expiresAt === null) {
      return;
    }
    const timer = window.setTimeout(
      () => {
        live.current = null;
        setPhase((current) => (current.kind === "showing" ? { kind: "expired" } : current));
      },
      Math.max(0, expiresAt - Date.now()),
    );
    return () => {
      window.clearTimeout(timer);
    };
  }, [expiresAt]);

  async function approve(link: DeviceLink, deviceName: string) {
    try {
      await client.approveDeviceLink(link.id);
      live.current = null;
      setPhase({ kind: "approved", deviceName });
    } catch (e) {
      live.current = null;
      setPhase({ kind: "failed", message: e instanceof ApiProblemError ? e.message : String(e) });
    }
  }

  const newCode = (
    <Button
      onPress={() => {
        setPhase({ kind: "starting" });
        void makeCode().then(show);
      }}
      className={primaryButtonClass + " flex items-center gap-1.5"}
    >
      <ArrowClockwiseIcon size={16} aria-hidden="true" />
      {m.deviceLink.newCode}
    </Button>
  );

  if (phase.kind === "scanned" && offering) {
    return (
      <div className="flex flex-col gap-3">
        <p className="text-sm font-medium">
          {format(m.deviceLink.confirmOffer, { device: phase.deviceName })}
        </p>
        <Caution text={m.deviceLink.confirmOfferWarning} />
        <div className="flex flex-wrap justify-end gap-2">
          <Button
            onPress={() => {
              void client.cancelDeviceLink(phase.link.id).catch(() => undefined);
              live.current = null;
              setPhase({ kind: "expired" });
            }}
            className={secondaryButtonClass}
          >
            {m.deviceLink.decline}
          </Button>
          <Button
            onPress={() => void approve(phase.link, phase.deviceName)}
            className={primaryButtonClass}
          >
            {m.deviceLink.approve}
          </Button>
        </div>
      </div>
    );
  }
  if (phase.kind === "approved") {
    return (
      <p role="status" className="text-sm">
        {format(m.deviceLink.approvedOffer, { device: phase.deviceName })}
      </p>
    );
  }
  if (phase.kind === "failed") {
    return (
      <div className="flex flex-col items-start gap-3">
        <p role="alert" className="rounded-md bg-danger-soft px-3 py-2 text-sm text-danger">
          {phase.message}
        </p>
        {newCode}
      </div>
    );
  }

  const shown = phase.kind === "showing" ? phase.link : null;
  return (
    <div className="flex flex-col gap-3">
      <p className="text-sm text-ink-muted">
        {offering ? m.deviceLink.offerHint : m.deviceLink.requestHint}
      </p>
      <Caution text={m.deviceLink.codeIsSecret} />
      {phase.kind === "scanned" ? (
        <p role="status" className="text-sm font-medium">
          {format(m.deviceLink.confirmOnPhone, { device: phase.deviceName })}
        </p>
      ) : shown !== null && share !== null ? (
        <QrCode
          text={share(deviceLinkPath(client.baseUrl, shown.id))}
          label={m.deviceLink.codeLabel}
          size={CODE_PX}
        />
      ) : phase.kind === "expired" ? (
        <QrCode
          // A stand-in drawn under the blur: the expired code itself is gone.
          text={share?.("/device-link") ?? "/device-link"}
          label={m.deviceLink.codeLabel}
          size={CODE_PX}
          cover={
            <>
              <p className="rounded bg-surface-raised/90 px-2 py-1 text-sm font-medium text-ink">
                {m.deviceLink.expired}
              </p>
              {newCode}
            </>
          }
        />
      ) : (
        <div aria-busy="true" className="flex flex-col items-center gap-1">
          <LoadingLabel text={m.deviceLink.making} />
          {/* `CODE_PX` across, as the code it stands for. */}
          <Skeleton className="size-56 rounded-md" />
        </div>
      )}
      {secondsLeft !== null && (
        <p className="text-center text-xs text-ink-muted tabular-nums">
          {format(m.deviceLink.expiresIn, { seconds: String(secondsLeft) })}
        </p>
      )}
    </div>
  );
}

/** A warning drawn so it is read: the code is a secret, or a stranger's code is a trap. */
export function Caution({ text }: { text: string }) {
  return (
    <p className="flex items-start gap-2 rounded-md border border-away/40 bg-surface px-3 py-2 text-sm">
      <WarningIcon size={18} aria-hidden="true" className="mt-0.5 shrink-0 text-away" />
      <span>{text}</span>
    </p>
  );
}
