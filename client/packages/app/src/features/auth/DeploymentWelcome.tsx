import { useState } from "react";
import { Button } from "react-aria-components";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";
import { AspenIcon, ICON_PX, iconSizeClass } from "./AspenIcon";
import { useDeploymentProfile } from "./deploymentProfile";
import { linkButtonClass } from "./styles";

/**
 * What the signed-out screens open with: the deployment's icon, or Aspen's when it has none or
 * its own fails to load, a welcome
 * naming it, or a general welcome when it has no name, and the server's address, with a way to
 * change it where the app can (`changeServer`; not on the web, whose server is the one serving
 * the page). The profile is read each time the screen opens, so a change shows at once; until
 * it arrives the icon and welcome wait, rather than showing Aspen's and the general one and then
 * the deployment's own, and a failed read shows Aspen's icon and the general welcome.
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
  const own = profile?.icon != null && !iconFailed ? profile.icon : null;
  return (
    <div className="flex w-full max-w-sm flex-col items-center gap-3 text-center">
      {own !== null ? (
        <img
          src={own.downloadUrl}
          alt=""
          width={ICON_PX}
          height={ICON_PX}
          onError={() => {
            setIconFailed(true);
          }}
          className={`${iconSizeClass} rounded-full object-cover select-none`}
        />
      ) : (
        profile !== undefined && <AspenIcon />
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
