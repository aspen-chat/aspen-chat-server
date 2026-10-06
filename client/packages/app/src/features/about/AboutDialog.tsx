import { Capacitor } from "@capacitor/core";
import { CLIENT_PROTOCOL, type AuthMethods, type Protocol } from "@aspen/protocol";
import { InfoIcon } from "@phosphor-icons/react";
import { Link } from "@tanstack/react-router";
import { useEffect, useState, type ReactNode } from "react";
import { Button, Dialog, DialogTrigger, Modal, ModalOverlay } from "react-aria-components";
import build from "virtual:build-info";
import aspenIcon from "../../../../../brand/aspen-icon.svg";
import { useAspenClient } from "@/api/context";
import { detectShell } from "@/config";
import { authMethods } from "@/features/auth/authMethods";
import { useDeploymentProfile } from "@/features/auth/deploymentProfile";
import { linkButtonClass } from "@/features/auth/styles";
import {
  dialogClass,
  overlayClass,
  planeClass,
  planesModalClass,
  secondaryButtonClass,
} from "@/features/invites/dialog";
import { DialogHeading } from "@/features/layout/DialogHeading";
import { LoadingLabel, Skeleton } from "@/features/layout/Skeleton";
import { useMessages } from "@/i18n/context";
import { format, type Messages } from "@/i18n/messages";

/**
 * About Aspen, opened from Settings by a button of its own: Aspen's icon, the deployment the user
 * is on (its name, or its address when it has none), the versions of everything this copy of
 * Aspen is made of (the app and the commit it was built from, the desktop or phone app around
 * it, the server's software, and the protocol versions each side speaks), the way to the Open
 * Source Attributions page, which `onNavigate` is told of so Settings can close, and, last, what
 * the Mozilla Public License lets the user do, with the license and the source code this build
 * was made from.
 */
export function AboutDialog({ onNavigate }: { onNavigate: () => void }) {
  const m = useMessages();
  return (
    <DialogTrigger>
      <Button className={secondaryButtonClass + " flex items-center gap-1.5"}>
        <InfoIcon size={16} aria-hidden="true" />
        {m.about.heading}
      </Button>
      <ModalOverlay isDismissable className={overlayClass}>
        <Modal className={planesModalClass}>
          <Dialog className={dialogClass}>
            <DialogHeading>{m.about.heading}</DialogHeading>
            <AboutPanel onNavigate={onNavigate} />
          </Dialog>
        </Modal>
      </ModalOverlay>
    </DialogTrigger>
  );
}

function AboutPanel({ onNavigate }: { onNavigate: () => void }) {
  const m = useMessages();
  const profile = useDeploymentProfile();
  const server = useServerVersions();
  const deployment =
    profile === undefined ? undefined : (profile.displayName ?? new URL(profile.webClientUrl).host);
  return (
    <>
      <section aria-labelledby="about-versions" className={planeClass}>
        <h3 id="about-versions" className="sr-only">
          {m.about.versions}
        </h3>
        <div className="flex items-center gap-3">
          <img src={aspenIcon} alt="" width={48} height={48} className="size-12 select-none" />
          <div className="flex min-w-0 flex-col">
            <p className="text-lg font-semibold">{m.appName}</p>
            <p className="text-sm break-words text-ink-muted" aria-busy={deployment === undefined}>
              {deployment === undefined ? (
                <>
                  <Skeleton inline className="w-40" />
                  <LoadingLabel />
                </>
              ) : (
                format(m.about.onDeployment, { name: deployment })
              )}
            </p>
          </div>
        </div>
        <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-sm">
          <Version label={m.about.app}>
            {build.commit === null
              ? build.version
              : format(build.modified ? m.about.appVersionModified : m.about.appVersion, {
                  version: build.version,
                  commit: build.commit,
                })}
          </Version>
          <ShellVersion />
          <Version label={m.about.server}>
            {server === undefined ? (
              <Pending />
            ) : server === null ? (
              m.about.serverUnknown
            ) : (
              `${server.software.name} ${server.software.version}`
            )}
          </Version>
          <Version label={m.about.protocol}>
            {server === undefined ? (
              <Pending />
            ) : (
              format(m.about.protocolVersions, {
                app: protocolRange(CLIENT_PROTOCOL, m),
                server: server === null ? "?" : protocolRange(server.protocol, m),
              })
            )}
          </Version>
        </dl>
        <Link to="/attributions" onClick={onNavigate} className={linkButtonClass + " self-start"}>
          {m.about.attributions}
        </Link>
      </section>
      <section aria-labelledby="about-rights" className={planeClass}>
        <h3 id="about-rights" className="text-sm font-semibold text-ink-muted">
          {m.about.yourRights}
        </h3>
        <p className="text-sm">{m.about.yourRightsSummary}</p>
        <p className="flex flex-wrap gap-x-4 gap-y-1 text-sm">
          <a href={LICENSE_URL} target="_blank" rel="noreferrer" className={linkButtonClass}>
            {m.about.readLicense}
          </a>
          <a href={build.source} target="_blank" rel="noreferrer" className={linkButtonClass}>
            {m.about.sourceCode}
          </a>
        </p>
      </section>
    </>
  );
}

/** The Mozilla Public License, version 2.0, as Mozilla publishes it. */
const LICENSE_URL = "https://www.mozilla.org/MPL/2.0/";

function Version({ label, children }: { label: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-ink-muted">{label}</dt>
      <dd className="font-mono break-words">{children}</dd>
    </>
  );
}

function Pending() {
  return (
    <span aria-busy="true">
      <Skeleton inline className="w-24" />
      <LoadingLabel />
    </span>
  );
}

/** One version, or the range from the oldest a side still speaks to the newest it knows. */
function protocolRange(protocol: Protocol, m: Messages): string {
  return protocol.minimum === protocol.version
    ? String(protocol.version)
    : format(m.about.protocolRange, {
        minimum: String(protocol.minimum),
        version: String(protocol.version),
      });
}

/**
 * The server's software and protocol (`GET /auth/methods`); `undefined` while it is asked and
 * `null` when it could not be.
 */
function useServerVersions(): AuthMethods | null | undefined {
  const client = useAspenClient();
  const [read, setRead] = useState<{ server: string; methods: AuthMethods | null } | undefined>();
  useEffect(() => {
    let current = true;
    const settle = (methods: AuthMethods | null) => {
      if (current) {
        setRead({ server: client.baseUrl, methods });
      }
    };
    authMethods(client).then(settle, () => {
      settle(null);
    });
    return () => {
      current = false;
    };
  }, [client]);
  return read?.server === client.baseUrl ? read.methods : undefined;
}

/** What wraps the app, when something does: the desktop app's Electron, or the phone app. */
function ShellVersion() {
  const m = useMessages();
  const shell = detectShell();
  // `undefined` while the app is asked, and `null` when it could not say.
  const [phone, setPhone] = useState<{ version: string; build: string } | null | undefined>();
  useEffect(() => {
    if (shell !== "mobile") {
      return;
    }
    let current = true;
    // Loaded only in the phone apps, which alone have it.
    import("@capacitor/app")
      .then(({ App }) => App.getInfo())
      .then(
        (info) => {
          if (current) {
            setPhone({ version: info.version, build: info.build });
          }
        },
        () => {
          if (current) {
            setPhone(null);
          }
        },
      );
    return () => {
      current = false;
    };
  }, [shell]);
  const desktop = window.aspenDesktop;
  if (desktop !== undefined) {
    return (
      <Version label={m.about.desktop}>{format(m.about.desktopVersion, desktop.versions)}</Version>
    );
  }
  if (shell !== "mobile" || phone === null) {
    return null;
  }
  return (
    <Version label={m.about.phone}>
      {phone === undefined ? (
        <Pending />
      ) : (
        format(m.about.phoneVersion, {
          platform: Capacitor.getPlatform() === "ios" ? m.about.ios : m.about.android,
          version: phone.version,
          build: phone.build,
        })
      )}
    </Version>
  );
}
