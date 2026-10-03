import type { DeploymentProfile } from "@aspen/protocol";
import { useEffect, useState } from "react";
import { Button } from "react-aria-components";
import { useAspenClient } from "@/api/context";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { linkButtonClass } from "./styles";

/** The side of the deployment's icon on the sign-in screens, in CSS pixels. */
const ICON_PX = 256;

/**
 * What the signed-out screens open with: the deployment's icon, when it has one, a welcome
 * naming it, or a general welcome when it has no name, and the server's address, with a way to
 * change it where the app can (`changeServer`; not on the web, whose server is the one serving
 * the page). The profile is read each time the screen opens, so a change shows at once; until
 * it arrives the welcome waits, rather than showing the general one and then the name, and a
 * failed read shows the general one.
 */
export function DeploymentWelcome({
  serverUrl,
  onChangeServer,
}: {
  serverUrl: string;
  onChangeServer: (() => void) | null;
}) {
  const m = useMessages();
  const profile = useDeploymentProfile();
  const [iconFailed, setIconFailed] = useState(false);
  const icon = profile === undefined || iconFailed ? null : profile.icon;
  return (
    <div className="flex w-full max-w-sm flex-col items-center gap-3 text-center">
      {icon != null && (
        <img
          src={icon.downloadUrl}
          alt=""
          width={ICON_PX}
          height={ICON_PX}
          onError={() => {
            setIconFailed(true);
          }}
          className="aspect-square max-w-full rounded-full object-cover select-none"
        />
      )}
      {profile !== undefined && (
        <p className="text-xl font-semibold text-balance break-words">
          {profile.displayName == null
            ? m.welcomeUnnamed
            : format(m.welcomeNamed, { name: profile.displayName })}
        </p>
      )}
      <p className="flex flex-wrap items-baseline justify-center gap-x-2 text-sm text-ink-muted">
        <span>
          {m.serverLabel}: <span className="font-mono break-all">{serverUrl}</span>
        </span>
        {onChangeServer !== null && (
          <Button onPress={onChangeServer} className={linkButtonClass}>
            {m.changeServer}
          </Button>
        )}
      </p>
    </div>
  );
}

/** The deployment's profile; `undefined` until it is read, and empty when the read fails. */
function useDeploymentProfile(): DeploymentProfile | undefined {
  const client = useAspenClient();
  const [read, setRead] = useState<{ server: string; profile: DeploymentProfile } | undefined>();
  useEffect(() => {
    let current = true;
    const settle = (profile: DeploymentProfile) => {
      if (current) {
        setRead({ server: client.baseUrl, profile });
      }
    };
    client.deploymentProfile().then(settle, () => {
      settle({ displayName: null, icon: null });
    });
    return () => {
      current = false;
    };
  }, [client]);
  return read?.server === client.baseUrl ? read.profile : undefined;
}
