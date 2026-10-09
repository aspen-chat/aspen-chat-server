import { ApiProblemError } from "@aspen/protocol";
import { useState } from "react";
import { Button } from "react-aria-components";
import { useDeploymentsHub, useForeignDeployments } from "@/api/deploymentsContext";
import { dangerButtonClass, planeClass, secondaryButtonClass } from "@/features/invites/dialog";
import { useMessages } from "@/i18n/context";
import { format } from "@/i18n/messages";

/**
 * The other servers the user signs in to with their home account, each to leave: signing out
 * there, and off the list their other devices follow. Shown only when there are any.
 */
export function OtherServersSection() {
  const m = useMessages();
  const servers = useForeignDeployments();
  if (servers.length === 0) {
    return null;
  }
  return (
    <section aria-labelledby="settings-servers" className={planeClass}>
      <h3 id="settings-servers" className="text-lg font-semibold text-ink-muted">
        {m.deployments.otherServers}
      </h3>
      <ul className="flex flex-col gap-1">
        {servers.map((server) => (
          <ServerRow key={server.domain} domain={server.domain} problem={server.problem} />
        ))}
      </ul>
    </section>
  );
}

function ServerRow({ domain, problem }: { domain: string; problem: string | null }) {
  const m = useMessages();
  const hub = useDeploymentsHub();
  const [confirming, setConfirming] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <li className="flex flex-col gap-1 rounded-md border border-line px-3 py-2">
      <span className="flex items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">{domain}</span>
        <Button
          isDisabled={pending}
          aria-label={format(m.deployments.leaveLabel, { domain })}
          onPress={() => {
            if (!confirming) {
              setConfirming(true);
              return;
            }
            setPending(true);
            setError(null);
            hub.leave(domain).catch((e: unknown) => {
              setError(e instanceof ApiProblemError ? e.message : String(e));
              setPending(false);
            });
          }}
          className={confirming ? dangerButtonClass : secondaryButtonClass + " text-danger"}
        >
          {confirming ? format(m.deployments.leaveConfirm, { domain }) : m.deployments.leave}
        </Button>
      </span>
      {problem !== null && <span className="text-xs text-danger">{problem}</span>}
      {error !== null && (
        <span role="alert" className="text-xs text-danger">
          {error}
        </span>
      )}
    </li>
  );
}
